//! Account diff types for DA encoding.

use alloy_primitives::U256;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{DaRegister, DaWrite};

use crate::{
    block::AccountSnapshot,
    codec::{CodecB256, CodecU256},
};

/// Diff for a single account using DA framework primitives.
///
/// - `balance`: Register (can change arbitrarily)
/// - `nonce`: Stored as `Option<u8>` (nonces typically increment by small amounts)
/// - `code_hash`: Register (only changes on contract creation)
#[derive(Clone, Debug, Default)]
pub struct AccountDiff {
    /// Balance change (full replacement if changed).
    pub balance: DaRegister<CodecU256>,
    /// Nonce increment (None = unchanged, Some(n) = increment by n).
    pub nonce_incr: Option<u8>,
    /// Code hash change (only on contract creation).
    pub code_hash: DaRegister<CodecB256>,
}

/// Converts a nonce delta to `Option<u8>`, panicking if it exceeds `u8::MAX`.
///
/// In practice, a single batch should never have more than 255 transactions
/// from the same account.
fn checked_nonce_incr(delta: u64, addr: Address) -> Option<u8> {
    if delta == 0 {
        return None;
    }
    assert!(
        delta <= u8::MAX as u64,
        "nonce delta {delta} exceeds u8::MAX for account {addr}",
    );
    Some(delta as u8)
}

impl AccountDiff {
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

    /// Creates a diff from block-level account states.
    ///
    /// If `original` is None, all fields are treated as changed (account creation).
    /// Returns None if no fields changed.
    pub fn from_account_snapshot(
        current: &AccountSnapshot,
        original: Option<&AccountSnapshot>,
        addr: Address,
    ) -> Option<Self> {
        let (orig_balance, orig_nonce, orig_code_hash) = original
            .map(|o| (Some(o.balance), o.nonce, Some(o.code_hash)))
            .unwrap_or((None, 0, None));

        let balance = match orig_balance {
            Some(ob) if ob == current.balance => DaRegister::new_unset(),
            _ => DaRegister::new_set(CodecU256(current.balance)),
        };

        let nonce_delta = current.nonce.saturating_sub(orig_nonce);
        let nonce_incr = checked_nonce_incr(nonce_delta, addr);

        let code_hash = match orig_code_hash {
            Some(oc) if oc == current.code_hash => DaRegister::new_unset(),
            _ if current.code_hash == KECCAK_EMPTY => DaRegister::new_unset(),
            _ => DaRegister::new_set(CodecB256(current.code_hash)),
        };

        let diff = Self {
            balance,
            nonce_incr,
            code_hash,
        };

        (!diff.is_unchanged()).then_some(diff)
    }

    /// Returns true if no changes are recorded.
    pub fn is_unchanged(&self) -> bool {
        DaWrite::is_default(&self.balance)
            && self.nonce_incr.is_none()
            && DaWrite::is_default(&self.code_hash)
    }
}

impl Codec for AccountDiff {
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
pub enum AccountChange {
    /// Account was created (new account).
    Created(AccountDiff),
    /// Account was updated (existing account modified).
    Updated(AccountDiff),
    /// Account was deleted (selfdestructed).
    Deleted,
}

impl AccountChange {
    /// Returns true if this is an empty/no-op change.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Updated(diff) => diff.is_unchanged(),
            _ => false,
        }
    }
}

impl Codec for AccountChange {
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
            0 => Ok(Self::Created(AccountDiff::decode(dec)?)),
            1 => Ok(Self::Updated(AccountDiff::decode(dec)?)),
            2 => Ok(Self::Deleted),
            _ => Err(CodecError::InvalidVariant("AccountChange")),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_account_diff_unchanged() {
        let diff = AccountDiff::new_unchanged();
        assert!(diff.is_unchanged());

        let encoded = encode_to_vec(&diff).unwrap();
        // Should just be 1 byte (bitmap = 0)
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0], 0);

        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();
        assert!(decoded.is_unchanged());
    }

    #[test]
    fn test_account_diff_created() {
        let diff = AccountDiff::new_created(U256::from(1000), 1, B256::from([0x11u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.balance.new_value().unwrap().0, U256::from(1000));
        assert_eq!(decoded.nonce_incr, Some(1u8));
        assert_eq!(
            decoded.code_hash.new_value().unwrap().0,
            B256::from([0x11u8; 32])
        );
    }

    #[test]
    fn test_account_change_roundtrip() {
        let created =
            AccountChange::Created(AccountDiff::new_created(U256::from(1000), 1, B256::ZERO));
        let updated = AccountChange::Updated(AccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(500))),
            nonce_incr: None,
            code_hash: DaRegister::new_unset(),
        });
        let deleted = AccountChange::Deleted;

        for change in [created, updated, deleted] {
            let encoded = encode_to_vec(&change).unwrap();
            let decoded: AccountChange = decode_buf_exact(&encoded).unwrap();

            // Verify tag matches
            match (&change, &decoded) {
                (AccountChange::Created(_), AccountChange::Created(_)) => {}
                (AccountChange::Updated(_), AccountChange::Updated(_)) => {}
                (AccountChange::Deleted, AccountChange::Deleted) => {}
                _ => panic!("Tag mismatch"),
            }
        }
    }
}
