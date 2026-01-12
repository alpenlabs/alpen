//! Serde-friendly types for RPC serialization.
//!
//! These types provide clean JSON representations of the DA state diff types.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use revm_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use strata_da_framework::DaRegister;

use crate::{
    account::{DaAccountChange, DaAccountDiff},
    codec::{CodecB256, CodecU256},
    diff::DaEeStateDiff,
    storage::DaAccountStorageDiff,
};

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

/// Serde-friendly representation of DaEeStateDiff for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaEeStateDiffSerde {
    /// Account changes, sorted by address.
    pub accounts: BTreeMap<Address, DaAccountChangeSerde>,
    /// Storage slot changes per account.
    pub storage: BTreeMap<Address, DaAccountStorageDiff>,
    /// Code hashes of deployed contracts.
    pub deployed_code_hashes: Vec<B256>,
}

impl From<&DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: &DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage.clone(),
            deployed_code_hashes: diff.deployed_code_hashes.clone(),
        }
    }
}

impl From<DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage,
            deployed_code_hashes: diff.deployed_code_hashes,
        }
    }
}

impl From<DaEeStateDiffSerde> for DaEeStateDiff {
    fn from(serde: DaEeStateDiffSerde) -> Self {
        Self {
            accounts: serde
                .accounts
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            storage: serde.storage,
            deployed_code_hashes: serde.deployed_code_hashes,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use revm_primitives::{Address, B256};

    use super::*;
    use crate::account::DaAccountDiff;

    #[test]
    fn test_account_diff_serde_roundtrip() {
        let diff = DaAccountDiff::new_created(U256::from(1000), 5, B256::from([0x11u8; 32]));

        // Convert to serde type
        let serde: DaAccountDiffSerde = (&diff).into();
        assert_eq!(serde.balance, Some(U256::from(1000)));
        assert_eq!(serde.nonce_incr, Some(5));
        assert_eq!(serde.code_hash, Some(B256::from([0x11u8; 32])));

        // Convert back
        let roundtrip: DaAccountDiff = serde.into();
        assert_eq!(roundtrip.balance.new_value().unwrap().0, U256::from(1000));
        assert_eq!(roundtrip.nonce_incr, Some(5));
        assert_eq!(
            roundtrip.code_hash.new_value().unwrap().0,
            B256::from([0x11u8; 32])
        );
    }

    #[test]
    fn test_account_change_serde_created() {
        let change =
            DaAccountChange::Created(DaAccountDiff::new_created(U256::from(500), 1, B256::ZERO));

        let serde: DaAccountChangeSerde = (&change).into();
        let json = serde_json::to_string(&serde).unwrap();
        assert!(json.contains(r#""type":"created""#));

        let roundtrip: DaAccountChange = serde.into();
        matches!(roundtrip, DaAccountChange::Created(_));
    }

    #[test]
    fn test_account_change_serde_deleted() {
        let change = DaAccountChange::Deleted;

        let serde: DaAccountChangeSerde = (&change).into();
        let json = serde_json::to_string(&serde).unwrap();
        assert!(json.contains(r#""type":"deleted""#));

        let roundtrip: DaAccountChange = serde.into();
        matches!(roundtrip, DaAccountChange::Deleted);
    }

    #[test]
    fn test_ee_state_diff_serde_json() {
        let mut diff = DaEeStateDiff::new();
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            DaAccountChange::Created(DaAccountDiff::new_created(U256::from(1000), 1, B256::ZERO)),
        );
        diff.deployed_code_hashes.push(B256::from([0x22u8; 32]));

        let serde: DaEeStateDiffSerde = (&diff).into();
        let json = serde_json::to_string_pretty(&serde).unwrap();

        // Verify JSON structure
        assert!(json.contains("accounts"));
        assert!(json.contains("storage"));
        assert!(json.contains("deployed_code_hashes"));

        // Deserialize back
        let parsed: DaEeStateDiffSerde = serde_json::from_str(&json).unwrap();
        let roundtrip: DaEeStateDiff = parsed.into();

        assert_eq!(roundtrip.accounts.len(), 1);
        assert_eq!(roundtrip.deployed_code_hashes.len(), 1);
    }
}
