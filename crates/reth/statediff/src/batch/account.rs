//! Account diff types for DA encoding.

use alloy_primitives::U256;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    counter_schemes::{CtrU64BySignedVarint, SignedVarintIncr},
    make_compound_impl, DaCounter, DaRegister, DaWrite,
};

use crate::{
    block::AccountSnapshot,
    codec::{CodecB256, CodecU256},
};

/// Diff for a single account using DA framework primitives.
///
/// - `balance`: Register (can change arbitrarily)
/// - `nonce`: Counter (signed delta, varint-encoded)
/// - `code_hash`: Register (only changes on contract creation)
///
/// # Why signed nonce deltas?
///
/// Post-Shanghai, account nonces can effectively decrease via the selfdestruct + recreate
/// pattern: when a contract selfdestructs and is recreated in the same block (or batch),
/// the new account starts with nonce 0 or 1, which may be lower than the original nonce.
/// Using signed deltas allows encoding these transitions compactly.
#[derive(Clone, Debug, Default)]
pub struct AccountDiff {
    /// Balance change (full replacement if changed).
    pub balance: DaRegister<CodecU256>,
    /// Nonce delta (signed, supports both increments and decrements).
    pub nonce: DaCounter<CtrU64BySignedVarint>,
    /// Code hash change (only on contract creation).
    pub code_hash: DaRegister<CodecB256>,
}

// Generate Codec and DaWrite impls via compound macro.
// Uses type coercion for balance (CodecU256 => U256) and code_hash (CodecB256 => B256).
make_compound_impl! {
    AccountDiff u8 => AccountSnapshot {
        balance: register [CodecU256 => U256],
        nonce: counter (CtrU64BySignedVarint),
        code_hash: register [CodecB256 => B256],
    }
}

/// Converts a nonce delta to `DaCounter<CtrU64BySignedVarint>`.
///
/// Uses signed varint encoding to handle nonce changes in either direction.
/// Nonces can decrease post-Shanghai via selfdestruct + recreate patterns.
///
/// Returns `None` if the delta exceeds the signed varint range (~±536 million).
fn nonce_delta_to_counter(delta: i64) -> Option<DaCounter<CtrU64BySignedVarint>> {
    if delta == 0 {
        return Some(DaCounter::new_unchanged());
    }
    let delta_i32 = i32::try_from(delta).ok()?;
    let incr = SignedVarintIncr::new(delta_i32)?;
    Some(DaCounter::new_changed(incr))
}

impl AccountDiff {
    /// Creates a new account diff with all fields unchanged.
    pub fn new_unchanged() -> Self {
        Self::default()
    }

    /// Creates a diff representing a new account creation.
    ///
    /// # Panics
    /// Panics if `nonce` exceeds the signed varint max (~536 million).
    pub fn new_created(balance: U256, nonce: u64, code_hash: B256) -> Self {
        let nonce_i32 = i32::try_from(nonce).expect("nonce exceeds i32::MAX");
        let incr = SignedVarintIncr::new(nonce_i32).expect("nonce exceeds signed varint max");
        Self {
            balance: DaRegister::new_set(CodecU256(balance)),
            nonce: DaCounter::new_changed(incr),
            code_hash: DaRegister::new_set(CodecB256(code_hash)),
        }
    }

    /// Creates a diff from block-level account states.
    ///
    /// If `original` is None, all fields are treated as changed (account creation).
    /// Returns None if no fields changed or if the nonce delta is invalid.
    ///
    /// Note: nonce deltas can be negative post-Shanghai due to selfdestruct + recreate.
    pub fn from_account_snapshot(
        current: &AccountSnapshot,
        original: Option<&AccountSnapshot>,
        _addr: Address,
    ) -> Option<Self> {
        let (orig_balance, orig_nonce, orig_code_hash) = original
            .map(|o| (Some(o.balance), o.nonce, Some(o.code_hash)))
            .unwrap_or((None, 0, None));

        let balance = match orig_balance {
            Some(ob) if ob == current.balance => DaRegister::new_unset(),
            _ => DaRegister::new_set(CodecU256(current.balance)),
        };

        // Signed delta: can be negative if account was selfdestructed and recreated
        let nonce_delta = (current.nonce as i64) - (orig_nonce as i64);
        let nonce = nonce_delta_to_counter(nonce_delta)?;

        let code_hash = match orig_code_hash {
            Some(oc) if oc == current.code_hash => DaRegister::new_unset(),
            _ if current.code_hash == KECCAK_EMPTY => DaRegister::new_unset(),
            _ => DaRegister::new_set(CodecB256(current.code_hash)),
        };

        let diff = Self {
            balance,
            nonce,
            code_hash,
        };

        (!diff.is_unchanged()).then_some(diff)
    }

    /// Returns true if no changes are recorded.
    pub fn is_unchanged(&self) -> bool {
        DaWrite::is_default(self)
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
    use strata_da_framework::ContextlessDaWrite;

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
        assert_eq!(decoded.nonce.diff().map(|v| v.inner()), Some(1));
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
            nonce: DaCounter::new_unchanged(),
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

    #[test]
    fn test_account_diff_apply() {
        let mut snapshot = AccountSnapshot {
            balance: U256::from(100),
            nonce: 5,
            code_hash: B256::ZERO,
        };

        let diff = AccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(200))),
            nonce: DaCounter::new_changed(SignedVarintIncr::new(3).unwrap()),
            code_hash: DaRegister::new_unset(),
        };

        ContextlessDaWrite::apply(&diff, &mut snapshot).unwrap();

        assert_eq!(snapshot.balance, U256::from(200));
        assert_eq!(snapshot.nonce, 8); // 5 + 3
        assert_eq!(snapshot.code_hash, B256::ZERO); // unchanged
    }

    #[test]
    fn test_account_diff_large_nonce_increment() {
        // Test with a value that would overflow u8 (>255)
        let diff = AccountDiff::new_created(U256::from(1000), 500, B256::ZERO);

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.nonce.diff().map(|v| v.inner()), Some(500));
    }

    #[test]
    fn test_account_diff_negative_nonce_delta() {
        // Post-Shanghai: selfdestruct + recreate can result in negative nonce delta.
        // Example: account had nonce 100, selfdestructs, gets recreated with nonce 1.

        let original = AccountSnapshot {
            balance: U256::from(1000),
            nonce: 100,
            code_hash: B256::ZERO,
        };

        let current = AccountSnapshot {
            balance: U256::from(500),
            nonce: 1, // Recreated with lower nonce
            code_hash: B256::from([0x11u8; 32]),
        };

        let diff =
            AccountDiff::from_account_snapshot(&current, Some(&original), Address::ZERO).unwrap();

        // Nonce delta should be -99 (1 - 100)
        assert_eq!(diff.nonce.diff().map(|v| v.inner()), Some(-99));

        // Verify encoding roundtrip
        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.nonce.diff().map(|v| v.inner()), Some(-99));

        // Verify apply works correctly
        let mut snapshot = original.clone();
        ContextlessDaWrite::apply(&decoded, &mut snapshot).unwrap();
        assert_eq!(snapshot.nonce, 1);
        assert_eq!(snapshot.balance, U256::from(500));
    }
}
