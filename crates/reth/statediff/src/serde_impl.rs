//! Serde-friendly types for RPC serialization.
//!
//! These types provide clean JSON representations of the batch state diff types.

use std::collections::BTreeMap;

use alloy_primitives::{Bytes, U256};
use revm_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use strata_da_framework::{
    counter_schemes::CtrU64BySignedVarInt, DaCounter, DaRegister, SignedVarInt,
};

use crate::{
    batch::{AccountChange, AccountDiff, BatchStateDiff, StorageDiff},
    codec::{CodecB256, CtrU256BySignedU256, SignedU256Delta},
};

/// Serde-friendly representation of a signed balance delta.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceDeltaSerde {
    /// True if positive (balance increased), false if negative (balance decreased).
    pub positive: bool,
    /// Absolute magnitude of the change.
    pub magnitude: U256,
}

/// Serde-friendly representation of [`AccountDiff`] for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountDiffSerde {
    /// Balance delta (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_delta: Option<BalanceDeltaSerde>,
    /// Nonce delta (None = unchanged). Can be negative post-Shanghai via selfdestruct+recreate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce_delta: Option<i64>,
    /// New code hash (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_hash: Option<B256>,
}

impl From<&AccountDiff> for AccountDiffSerde {
    fn from(diff: &AccountDiff) -> Self {
        Self {
            balance_delta: diff.balance.diff().map(|d| BalanceDeltaSerde {
                positive: d.is_nonnegative(),
                magnitude: d.magnitude(),
            }),
            nonce_delta: diff.nonce.diff().and_then(|v| v.to_i64()),
            code_hash: diff.code_hash.new_value().map(|v| v.0),
        }
    }
}

impl From<AccountDiffSerde> for AccountDiff {
    fn from(serde: AccountDiffSerde) -> Self {
        Self {
            balance: serde
                .balance_delta
                .map(|d| {
                    let delta = if d.positive {
                        SignedU256Delta::positive(d.magnitude)
                    } else {
                        SignedU256Delta::negative(d.magnitude)
                    };
                    DaCounter::<CtrU256BySignedU256>::new_changed(delta)
                })
                .unwrap_or_else(DaCounter::new_unchanged),
            nonce: serde
                .nonce_delta
                .map(SignedVarInt::from_i64)
                .map(DaCounter::<CtrU64BySignedVarInt>::new_changed)
                .unwrap_or_else(DaCounter::new_unchanged),
            code_hash: serde
                .code_hash
                .map(|v| DaRegister::new_set(CodecB256(v)))
                .unwrap_or_else(DaRegister::new_unset),
        }
    }
}

/// Serde-friendly representation of [`AccountChange`] for RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AccountChangeSerde {
    Created(AccountDiffSerde),
    Updated(AccountDiffSerde),
    Deleted,
}

impl From<&AccountChange> for AccountChangeSerde {
    fn from(change: &AccountChange) -> Self {
        match change {
            AccountChange::Created(diff) => Self::Created(diff.into()),
            AccountChange::Updated(diff) => Self::Updated(diff.into()),
            AccountChange::Deleted => Self::Deleted,
        }
    }
}

impl From<AccountChangeSerde> for AccountChange {
    fn from(serde: AccountChangeSerde) -> Self {
        match serde {
            AccountChangeSerde::Created(diff) => Self::Created(diff.into()),
            AccountChangeSerde::Updated(diff) => Self::Updated(diff.into()),
            AccountChangeSerde::Deleted => Self::Deleted,
        }
    }
}

/// Serde-friendly representation of [`BatchStateDiff`] for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BatchStateDiffSerde {
    /// Account changes, sorted by address.
    pub accounts: BTreeMap<Address, AccountChangeSerde>,
    /// Storage slot changes per account.
    pub storage: BTreeMap<Address, StorageDiff>,
    /// Deployed contract bytecodes keyed by code hash (hex-encoded).
    pub deployed_bytecodes: BTreeMap<B256, Bytes>,
}

impl From<&BatchStateDiff> for BatchStateDiffSerde {
    fn from(diff: &BatchStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage.clone(),
            deployed_bytecodes: diff.deployed_bytecodes.clone(),
        }
    }
}

impl From<BatchStateDiff> for BatchStateDiffSerde {
    fn from(diff: BatchStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage,
            deployed_bytecodes: diff.deployed_bytecodes,
        }
    }
}

impl From<BatchStateDiffSerde> for BatchStateDiff {
    fn from(serde: BatchStateDiffSerde) -> Self {
        Self {
            accounts: serde
                .accounts
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            storage: serde.storage,
            deployed_bytecodes: serde.deployed_bytecodes,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use revm_primitives::{Address, B256};

    use super::*;

    #[test]
    fn test_account_diff_serde_roundtrip() {
        let diff = AccountDiff::new_created(U256::from(1000), 5, B256::from([0x11u8; 32]));

        // Convert to serde type
        let serde: AccountDiffSerde = (&diff).into();
        // Balance delta is +1000 (from 0 for new account)
        let balance_delta = serde.balance_delta.as_ref().unwrap();
        assert!(balance_delta.positive);
        assert_eq!(balance_delta.magnitude, U256::from(1000));
        assert_eq!(serde.nonce_delta, Some(5));
        assert_eq!(serde.code_hash, Some(B256::from([0x11u8; 32])));

        // Convert back
        let roundtrip: AccountDiff = serde.into();
        let roundtrip_delta = roundtrip.balance.diff().unwrap();
        assert!(roundtrip_delta.is_nonnegative());
        assert_eq!(roundtrip_delta.magnitude(), U256::from(1000));
        assert_eq!(roundtrip.nonce.diff().and_then(|v| v.to_i64()), Some(5));
        assert_eq!(
            roundtrip.code_hash.new_value().unwrap().0,
            B256::from([0x11u8; 32])
        );
    }

    #[test]
    fn test_account_change_serde_created() {
        let change =
            AccountChange::Created(AccountDiff::new_created(U256::from(500), 1, B256::ZERO));

        let serde: AccountChangeSerde = (&change).into();
        let json = serde_json::to_string(&serde).unwrap();
        assert!(json.contains(r#""type":"created""#));

        let roundtrip: AccountChange = serde.into();
        matches!(roundtrip, AccountChange::Created(_));
    }

    #[test]
    fn test_account_change_serde_deleted() {
        let change = AccountChange::Deleted;

        let serde: AccountChangeSerde = (&change).into();
        let json = serde_json::to_string(&serde).unwrap();
        assert!(json.contains(r#""type":"deleted""#));

        let roundtrip: AccountChange = serde.into();
        matches!(roundtrip, AccountChange::Deleted);
    }

    #[test]
    fn test_batch_state_diff_serde_json() {
        let mut diff = BatchStateDiff::new();
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            AccountChange::Created(AccountDiff::new_created(U256::from(1000), 1, B256::ZERO)),
        );
        diff.deployed_bytecodes.insert(
            B256::from([0x22u8; 32]),
            Bytes::from_static(&[0x60, 0x80, 0x60, 0x40]),
        );

        let serde: BatchStateDiffSerde = (&diff).into();
        let json = serde_json::to_string_pretty(&serde).unwrap();

        // Verify JSON structure
        assert!(json.contains("accounts"));
        assert!(json.contains("storage"));
        assert!(json.contains("deployed_bytecodes"));

        // Deserialize back
        let parsed: BatchStateDiffSerde = serde_json::from_str(&json).unwrap();
        let roundtrip: BatchStateDiff = parsed.into();

        assert_eq!(roundtrip.accounts.len(), 1);
        assert_eq!(roundtrip.deployed_bytecodes.len(), 1);
    }
}
