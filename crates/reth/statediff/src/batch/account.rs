//! Account diff types for DA encoding.

use alloy_primitives::U256;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    SignedVarInt, counter_schemes::CtrU64BySignedVarInt, make_compound_impl, DaCounter, DaRegister,
    DaWrite,
};

use crate::{
    block::AccountSnapshot,
    codec::{CodecB256, CtrU256BySignedU256, SignedU256Delta},
};

/// Diff for a single account using DA framework primitives.
///
/// - `balance`: Counter (signed U256 delta, trimmed encoding)
/// - `nonce`: Counter (signed delta, varint-encoded)
/// - `code_hash`: Register (only changes on contract creation)
///
/// # Delta vs Full Value Replacement
///
/// **Full value replacement** (register): Store the new value directly.
/// - Example: balance changes from 1000 to 1005 → encode `1005` (full 32 bytes for U256)
///
/// **Delta encoding** (counter): Store only the difference between old and new.
/// - Example: balance changes from 1000 to 1005 → encode `+5` (1 byte for sign + 1 byte for value)
///
/// # Why signed deltas?
///
/// **Balance**: Delta encoding provides significant space savings for typical transactions
/// where balance changes by small amounts (gas fees, small transfers) compared to the total
/// balance. The signed delta supports both increases (deposits, rewards) and decreases
/// (transfers, fees).
///
/// **Nonce**: Post-Shanghai, account nonces can effectively decrease via the selfdestruct +
/// recreate pattern: when a contract selfdestructs and is recreated in the same block (or batch),
/// the new account starts with nonce 0 or 1, which may be lower than the original nonce.
#[derive(Clone, Debug, Default)]
pub struct AccountDiff {
    /// Balance delta (signed, supports increases and decreases).
    pub balance: DaCounter<CtrU256BySignedU256>,
    /// Nonce delta (signed, supports both increments and decrements).
    pub nonce: DaCounter<CtrU64BySignedVarInt>,
    /// Code hash change (only on contract creation).
    pub code_hash: DaRegister<CodecB256>,
}

// Generate Codec and DaWrite impls via compound macro.
// Uses type coercion for code_hash (CodecB256 => B256).
make_compound_impl! {
    AccountDiff u8 => AccountSnapshot {
        balance: counter (CtrU256BySignedU256),
        nonce: counter (CtrU64BySignedVarInt),
        code_hash: register [CodecB256 => B256],
    }
}

/// Converts a nonce delta to `DaCounter<CtrU64BySignedVarInt>`.
///
/// Uses signed varint encoding to handle nonce changes in either direction.
/// Nonces can decrease post-Shanghai via selfdestruct + recreate patterns.
fn nonce_delta_to_counter(delta: i64) -> DaCounter<CtrU64BySignedVarInt> {
    if delta == 0 {
        return DaCounter::new_unchanged();
    }
    DaCounter::new_changed(SignedVarInt::from_i64(delta))
}

/// Converts a balance delta to `DaCounter<CtrU256BySignedU256>`.
///
/// Computes the signed difference between old and new balance.
fn balance_delta_to_counter(old: U256, new: U256) -> DaCounter<CtrU256BySignedU256> {
    if old == new {
        return DaCounter::new_unchanged();
    }
    let delta = if new > old {
        SignedU256Delta::positive(new - old)
    } else {
        SignedU256Delta::negative(old - new)
    };
    DaCounter::new_changed(delta)
}

impl AccountDiff {
    /// Creates a new account diff with all fields unchanged.
    pub fn new_unchanged() -> Self {
        Self::default()
    }

    /// Creates a diff representing a new account creation.
    pub fn new_created(balance: U256, nonce: u64, code_hash: B256) -> Self {
        let nonce_incr = SignedVarInt::positive(nonce);
        // For new accounts, balance delta is the full balance (from 0)
        let balance_delta = SignedU256Delta::positive(balance);
        Self {
            balance: DaCounter::new_changed(balance_delta),
            nonce: DaCounter::new_changed(nonce_incr),
            code_hash: DaRegister::new_set(CodecB256(code_hash)),
        }
    }

    /// Creates a diff from two point-in-time account state ([`AccountSnapshot`]).
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
            .map(|o| (o.balance, o.nonce, Some(o.code_hash)))
            .unwrap_or((U256::ZERO, 0, None));

        // Balance delta (signed): can be positive or negative
        let balance = balance_delta_to_counter(orig_balance, current.balance);

        // Nonce delta (signed): can be negative if account was selfdestructed and recreated
        let nonce_delta = (current.nonce as i64) - (orig_nonce as i64);
        let nonce = nonce_delta_to_counter(nonce_delta);

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
    use strata_da_framework::{ContextlessDaWrite, SignedVarInt};

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

        // Balance delta should be +1000 (from 0)
        let balance_diff = decoded.balance.diff().unwrap();
        assert!(balance_diff.is_nonnegative());
        assert_eq!(balance_diff.magnitude(), U256::from(1000));
        assert_eq!(decoded.nonce.diff().and_then(|v| v.to_i64()), Some(1));
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
            balance: DaCounter::new_changed(SignedU256Delta::positive(U256::from(500))),
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
            // Balance increases by 100 (from 100 to 200)
            balance: DaCounter::new_changed(SignedU256Delta::positive(U256::from(100))),
            nonce: DaCounter::new_changed(SignedVarInt::positive(3)),
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

        assert_eq!(decoded.nonce.diff().and_then(|v| v.to_i64()), Some(500));
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
        assert_eq!(diff.nonce.diff().and_then(|v| v.to_i64()), Some(-99));

        // Verify encoding roundtrip
        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.nonce.diff().and_then(|v| v.to_i64()), Some(-99));

        // Verify apply works correctly
        let mut snapshot = original.clone();
        ContextlessDaWrite::apply(&decoded, &mut snapshot).unwrap();
        assert_eq!(snapshot.nonce, 1);
        assert_eq!(snapshot.balance, U256::from(500));
    }

    #[test]
    fn test_account_diff_negative_balance_delta() {
        // Balance decreased: e.g., transfer out or gas payment
        let original = AccountSnapshot {
            balance: U256::from(1_000_000),
            nonce: 5,
            code_hash: B256::ZERO,
        };

        let current = AccountSnapshot {
            balance: U256::from(999_000), // Decreased by 1000
            nonce: 6,
            code_hash: B256::ZERO,
        };

        let diff =
            AccountDiff::from_account_snapshot(&current, Some(&original), Address::ZERO).unwrap();

        // Balance delta should be negative (decrease of 1000)
        let balance_delta = diff.balance.diff().unwrap();
        assert!(!balance_delta.is_nonnegative());
        assert_eq!(balance_delta.magnitude(), U256::from(1000));

        // Verify encoding roundtrip
        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: AccountDiff = decode_buf_exact(&encoded).unwrap();

        let decoded_delta = decoded.balance.diff().unwrap();
        assert!(!decoded_delta.is_nonnegative());
        assert_eq!(decoded_delta.magnitude(), U256::from(1000));

        // Verify apply works correctly
        let mut snapshot = original.clone();
        ContextlessDaWrite::apply(&decoded, &mut snapshot).unwrap();
        assert_eq!(snapshot.balance, U256::from(999_000));
        assert_eq!(snapshot.nonce, 6);
    }
}
