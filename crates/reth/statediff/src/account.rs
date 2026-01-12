//! Account state and diff types for DA encoding.

use alloy_primitives::U256;
use revm_primitives::B256;
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{BuilderError, ContextlessDaWrite, DaError, DaRegister, DaWrite};

use crate::codec::{CodecB256, CodecU256};

/// Represents the EE account state that DA diffs are applied to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaAccountState {
    pub balance: CodecU256,
    pub nonce: u64,
    pub code_hash: CodecB256,
}

impl DaAccountState {
    pub fn new(balance: U256, nonce: u64, code_hash: B256) -> Self {
        Self {
            balance: CodecU256(balance),
            nonce,
            code_hash: CodecB256(code_hash),
        }
    }
}

/// Diff for a single account using DA framework primitives.
///
/// - `balance`: Register (can change arbitrarily)
/// - `nonce`: Stored as `Option<u8>` (nonces typically increment by small amounts)
/// - `code_hash`: Register (only changes on contract creation)
#[derive(Clone, Debug, Default)]
pub struct DaAccountDiff {
    /// Balance change (full replacement if changed).
    pub balance: DaRegister<CodecU256>,
    /// Nonce increment (None = unchanged, Some(n) = increment by n).
    pub nonce_incr: Option<u8>,
    /// Code hash change (only on contract creation).
    pub code_hash: DaRegister<CodecB256>,
}

/// Serde-friendly representation of DaAccountDiff for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaAccountDiffSerde {
    /// New balance value (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<U256>,
    /// Nonce increment (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce_incr: Option<u8>,
    /// New code hash (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_hash: Option<B256>,
}

impl From<&DaAccountDiff> for DaAccountDiffSerde {
    fn from(diff: &DaAccountDiff) -> Self {
        Self {
            balance: diff.balance.new_value().map(|v| v.0),
            nonce_incr: diff.nonce_incr,
            code_hash: diff.code_hash.new_value().map(|v| v.0),
        }
    }
}

impl From<DaAccountDiffSerde> for DaAccountDiff {
    fn from(serde: DaAccountDiffSerde) -> Self {
        Self {
            balance: serde
                .balance
                .map(|v| DaRegister::new_set(CodecU256(v)))
                .unwrap_or_else(DaRegister::new_unset),
            nonce_incr: serde.nonce_incr,
            code_hash: serde
                .code_hash
                .map(|v| DaRegister::new_set(CodecB256(v)))
                .unwrap_or_else(DaRegister::new_unset),
        }
    }
}

impl DaAccountDiff {
    /// Creates a new account diff with all fields unchanged.
    pub fn new_unchanged() -> Self {
        Self::default()
    }

    /// Creates a diff representing a new account creation.
    pub fn new_created(balance: U256, nonce: u64, code_hash: B256) -> Self {
        Self {
            balance: DaRegister::new_set(CodecU256(balance)),
            nonce_incr: if nonce > 0 { Some(nonce as u8) } else { None },
            code_hash: DaRegister::new_set(CodecB256(code_hash)),
        }
    }

    /// Creates a diff by comparing original and new account states.
    pub fn from_change(
        original: &DaAccountState,
        new: &DaAccountState,
    ) -> Result<Self, BuilderError> {
        let balance = DaRegister::compare(&original.balance, &new.balance);

        // For nonce, compute the increment
        let nonce_diff = new.nonce.saturating_sub(original.nonce);
        let nonce_incr = if nonce_diff == 0 {
            None
        } else if nonce_diff <= u8::MAX as u64 {
            Some(nonce_diff as u8)
        } else {
            return Err(BuilderError::OutOfBoundsValue);
        };

        let code_hash = DaRegister::compare(&original.code_hash, &new.code_hash);

        Ok(Self {
            balance,
            nonce_incr,
            code_hash,
        })
    }

    /// Returns true if no changes are recorded.
    pub fn is_unchanged(&self) -> bool {
        DaWrite::is_default(&self.balance)
            && self.nonce_incr.is_none()
            && DaWrite::is_default(&self.code_hash)
    }
}

impl DaWrite for DaAccountDiff {
    type Target = DaAccountState;
    type Context = ();

    fn is_default(&self) -> bool {
        self.is_unchanged()
    }

    fn apply(&self, target: &mut Self::Target, _context: &Self::Context) -> Result<(), DaError> {
        ContextlessDaWrite::apply(&self.balance, &mut target.balance)?;
        if let Some(incr) = self.nonce_incr {
            target.nonce = target.nonce.wrapping_add(incr as u64);
        }
        ContextlessDaWrite::apply(&self.code_hash, &mut target.code_hash)?;
        Ok(())
    }
}

impl Codec for DaAccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Use a bitmap to track which fields are set (3 bits needed)
        let mut bitmap: u8 = 0;
        if !DaWrite::is_default(&self.balance) {
            bitmap |= 1;
        }
        if self.nonce_incr.is_some() {
            bitmap |= 2;
        }
        if !DaWrite::is_default(&self.code_hash) {
            bitmap |= 4;
        }

        bitmap.encode(enc)?;

        // Only encode non-default fields
        if !DaWrite::is_default(&self.balance) {
            self.balance.new_value().unwrap().encode(enc)?;
        }
        if let Some(incr) = self.nonce_incr {
            incr.encode(enc)?;
        }
        if !DaWrite::is_default(&self.code_hash) {
            self.code_hash.new_value().unwrap().encode(enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let bitmap = u8::decode(dec)?;

        let balance = if bitmap & 1 != 0 {
            DaRegister::new_set(CodecU256::decode(dec)?)
        } else {
            DaRegister::new_unset()
        };

        let nonce_incr = if bitmap & 2 != 0 {
            Some(u8::decode(dec)?)
        } else {
            None
        };

        let code_hash = if bitmap & 4 != 0 {
            DaRegister::new_set(CodecB256::decode(dec)?)
        } else {
            DaRegister::new_unset()
        };

        Ok(Self {
            balance,
            nonce_incr,
            code_hash,
        })
    }
}

/// Represents the type of change to an account.
#[derive(Clone, Debug)]
pub enum DaAccountChange {
    /// Account was created (new account).
    Created(DaAccountDiff),
    /// Account was updated (existing account modified).
    Updated(DaAccountDiff),
    /// Account was deleted (selfdestructed).
    Deleted,
}

/// Serde-friendly representation of DaAccountChange for RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DaAccountChangeSerde {
    Created(DaAccountDiffSerde),
    Updated(DaAccountDiffSerde),
    Deleted,
}

impl From<&DaAccountChange> for DaAccountChangeSerde {
    fn from(change: &DaAccountChange) -> Self {
        match change {
            DaAccountChange::Created(diff) => Self::Created(diff.into()),
            DaAccountChange::Updated(diff) => Self::Updated(diff.into()),
            DaAccountChange::Deleted => Self::Deleted,
        }
    }
}

impl From<DaAccountChangeSerde> for DaAccountChange {
    fn from(serde: DaAccountChangeSerde) -> Self {
        match serde {
            DaAccountChangeSerde::Created(diff) => Self::Created(diff.into()),
            DaAccountChangeSerde::Updated(diff) => Self::Updated(diff.into()),
            DaAccountChangeSerde::Deleted => Self::Deleted,
        }
    }
}

impl DaAccountChange {
    /// Returns true if this is an empty/no-op change.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Updated(diff) => diff.is_unchanged(),
            _ => false,
        }
    }
}

impl Codec for DaAccountChange {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match self {
            Self::Created(diff) => {
                0u8.encode(enc)?;
                diff.encode(enc)?;
            }
            Self::Updated(diff) => {
                1u8.encode(enc)?;
                diff.encode(enc)?;
            }
            Self::Deleted => {
                2u8.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let tag = u8::decode(dec)?;
        match tag {
            0 => Ok(Self::Created(DaAccountDiff::decode(dec)?)),
            1 => Ok(Self::Updated(DaAccountDiff::decode(dec)?)),
            2 => Ok(Self::Deleted),
            _ => Err(CodecError::InvalidVariant("DaAccountChange")),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_account_diff_unchanged() {
        let diff = DaAccountDiff::new_unchanged();
        assert!(diff.is_unchanged());

        let encoded = encode_to_vec(&diff).unwrap();
        // Should just be 1 byte (bitmap = 0)
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0], 0);

        let decoded: DaAccountDiff = decode_buf_exact(&encoded).unwrap();
        assert!(decoded.is_unchanged());
    }

    #[test]
    fn test_account_diff_created() {
        let diff = DaAccountDiff::new_created(U256::from(1000), 1, B256::from([0x11u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaAccountDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.balance.new_value().unwrap().0, U256::from(1000));
        assert_eq!(decoded.nonce_incr, Some(1u8));
        assert_eq!(
            decoded.code_hash.new_value().unwrap().0,
            B256::from([0x11u8; 32])
        );
    }

    #[test]
    fn test_account_diff_from_change() {
        let original = DaAccountState::new(U256::from(1000), 5, B256::from([0x11u8; 32]));
        let new = DaAccountState::new(
            U256::from(2000),
            7,                        // +2 increment
            B256::from([0x11u8; 32]), // unchanged
        );

        let diff = DaAccountDiff::from_change(&original, &new).unwrap();

        // Balance changed
        assert!(!DaWrite::is_default(&diff.balance));
        assert_eq!(diff.balance.new_value().unwrap().0, U256::from(2000));

        // Nonce incremented by 2
        assert_eq!(diff.nonce_incr, Some(2u8));

        // Code hash unchanged
        assert!(DaWrite::is_default(&diff.code_hash));
    }

    #[test]
    fn test_account_diff_apply() {
        let mut state = DaAccountState::new(U256::from(1000), 5, B256::from([0x11u8; 32]));

        let diff = DaAccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(2000))),
            nonce_incr: Some(3),
            code_hash: DaRegister::new_unset(),
        };

        ContextlessDaWrite::apply(&diff, &mut state).unwrap();

        assert_eq!(state.balance.0, U256::from(2000));
        assert_eq!(state.nonce, 8); // 5 + 3
        assert_eq!(state.code_hash.0, B256::from([0x11u8; 32])); // unchanged
    }

    #[test]
    fn test_account_change_roundtrip() {
        let created =
            DaAccountChange::Created(DaAccountDiff::new_created(U256::from(1000), 1, B256::ZERO));
        let updated = DaAccountChange::Updated(DaAccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(500))),
            nonce_incr: None,
            code_hash: DaRegister::new_unset(),
        });
        let deleted = DaAccountChange::Deleted;

        for change in [created, updated, deleted] {
            let encoded = encode_to_vec(&change).unwrap();
            let decoded: DaAccountChange = decode_buf_exact(&encoded).unwrap();

            // Verify tag matches
            match (&change, &decoded) {
                (DaAccountChange::Created(_), DaAccountChange::Created(_)) => {}
                (DaAccountChange::Updated(_), DaAccountChange::Updated(_)) => {}
                (DaAccountChange::Deleted, DaAccountChange::Deleted) => {}
                _ => panic!("Tag mismatch"),
            }
        }
    }
}
