//! OL genesis parameters.
//!
//! Provides JSON-serializable configuration for OL genesis state, including
//! genesis block header parameters and genesis account definitions.
//!
//! ## Header parameters
//!
//! [`GenesisHeaderParams`] configures the genesis block header. All fields
//! default to zero values when omitted.
//!
//! ## Account parameters
//!
//! [`GenesisAccountsParams`] configures genesis OL accounts. At genesis
//! construction time, each account entry is used to build an [`OLAccountState`]
//! with auto-assigned serials starting at 128 (`SYSTEM_RESERVED_ACCTS`).
//!
//! Currently only **snark accounts** are supported. Empty accounts are not
//! supported in genesis configuration.
//!
//! [`OLAccountState`]: https://docs.rs/strata-ol-state-types

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_identifiers::{AccountId, Buf32, Epoch};
use strata_predicate::PredicateKey;

/// Genesis block header parameters.
///
/// All fields have sensible defaults for a genesis block. If not provided,
/// `timestamp` and `epoch` default to 0, while `parent_blkid`, `body_root`,
/// and `logs_root` default to `Buf32::zero()`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisHeaderParams {
    /// Block timestamp. Defaults to 0.
    #[serde(default)]
    pub timestamp: u64,

    /// Epoch number. Defaults to 0.
    #[serde(default)]
    pub epoch: Epoch,

    /// Parent block ID. Defaults to `Buf32::zero()`.
    #[serde(default = "Buf32::zero")]
    pub parent_blkid: Buf32,

    /// Body root hash. Defaults to `Buf32::zero()`.
    #[serde(default = "Buf32::zero")]
    pub body_root: Buf32,

    /// Logs root hash. Defaults to `Buf32::zero()`.
    #[serde(default = "Buf32::zero")]
    pub logs_root: Buf32,
}

impl GenesisHeaderParams {
    /// Deserializes from a JSON string.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serializes to a JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Serializes to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

/// Top-level OL genesis account parameters.
///
/// Currently only snark accounts are supported. Empty accounts are not
/// supported in genesis configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAccountsParams {
    /// Genesis accounts keyed by account ID.
    pub accounts: BTreeMap<AccountId, AccountParams>,
}

/// Parameters for a single genesis snark account.
///
/// The `predicate` and `inner_state` fields are required. The `balance` field
/// defaults to 0 if omitted. Other account fields (`serial`, `seqno`,
/// `inbox_mmr`, `next_msg_read_idx`) are auto-computed at genesis construction
/// time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountParams {
    /// Verifying key (predicate).
    pub predicate: PredicateKey,

    /// Inner state root commitment.
    pub inner_state: Buf32,

    /// Initial balance in satoshis. Defaults to 0.
    #[serde(default)]
    pub balance: u64,
}

impl GenesisAccountsParams {
    /// Deserializes from a JSON sting.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serializes to a JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Serializes to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params() -> GenesisAccountsParams {
        let mut accounts = BTreeMap::new();

        let id1 = AccountId::from([1u8; 32]);
        let id2 = AccountId::from([2u8; 32]);

        accounts.insert(
            id1,
            AccountParams {
                predicate: PredicateKey::always_accept(),
                inner_state: Buf32::zero(),
                balance: 1000,
            },
        );

        accounts.insert(
            id2,
            AccountParams {
                predicate: PredicateKey::always_accept(),
                inner_state: Buf32::from([0xab; 32]),
                balance: 0,
            },
        );

        GenesisAccountsParams { accounts }
    }

    #[test]
    fn test_json_roundtrip() {
        let params = sample_params();
        let json = params.to_json().expect("serialization failed");
        let decoded = GenesisAccountsParams::from_json(&json).expect("deserialization failed");

        assert_eq!(params.accounts.len(), decoded.accounts.len());
        for (id, original) in &params.accounts {
            let restored = decoded.accounts.get(id).expect("missing account");
            assert_eq!(original.balance, restored.balance);
            assert_eq!(original.inner_state, restored.inner_state);
        }
    }

    #[test]
    fn test_balance_defaults_to_zero() {
        // JSON with no balance field on the second account.
        let json = r#"{
            "accounts": {
                "0101010101010101010101010101010101010101010101010101010101010101": {
                    "predicate": "AlwaysAccept",
                    "inner_state": "0000000000000000000000000000000000000000000000000000000000000000",
                    "balance": 500
                },
                "0202020202020202020202020202020202020202020202020202020202020202": {
                    "predicate": "AlwaysAccept",
                    "inner_state": "abababababababababababababababababababababababababababababababab"
                }
            }
        }"#;

        let params = GenesisAccountsParams::from_json(json).expect("parse failed");
        assert_eq!(params.accounts.len(), 2);

        let id1 = AccountId::from([1u8; 32]);
        let id2 = AccountId::from([2u8; 32]);

        assert_eq!(params.accounts[&id1].balance, 500);
        assert_eq!(params.accounts[&id2].balance, 0);
    }

    #[test]
    fn test_empty_accounts_map() {
        let json = r#"{ "accounts": {} }"#;
        let params = GenesisAccountsParams::from_json(json).expect("parse failed");
        assert!(params.accounts.is_empty());
    }

    #[test]
    fn test_missing_required_field_errors() {
        // Missing inner_state.
        let json = r#"{
            "accounts": {
                "0101010101010101010101010101010101010101010101010101010101010101": {
                    "predicate": "AlwaysAccept"
                }
            }
        }"#;

        let result = GenesisAccountsParams::from_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_pretty_json_output() {
        let params = sample_params();
        let pretty = params
            .to_json_pretty()
            .expect("pretty serialization failed");
        assert!(pretty.contains('\n'));
        // Verify it round-trips.
        let decoded = GenesisAccountsParams::from_json(&pretty).expect("deserialization failed");
        assert_eq!(params.accounts.len(), decoded.accounts.len());
    }

    #[test]
    fn test_accounts_sorted_by_id() {
        let params = sample_params();
        let ids: Vec<_> = params.accounts.keys().collect();
        for window in ids.windows(2) {
            assert!(window[0] < window[1], "accounts should be sorted by ID");
        }
    }

    #[test]
    fn test_header_all_defaults() {
        let json = r#"{}"#;
        let params = GenesisHeaderParams::from_json(json).expect("parse failed");

        assert_eq!(params.timestamp, 0);
        assert_eq!(params.epoch, 0);
        assert_eq!(params.parent_blkid, Buf32::zero());
        assert_eq!(params.body_root, Buf32::zero());
        assert_eq!(params.logs_root, Buf32::zero());
    }

    #[test]
    fn test_header_explicit_values() {
        let json = r#"{
            "timestamp": 42,
            "epoch": 7,
            "parent_blkid": "0101010101010101010101010101010101010101010101010101010101010101",
            "body_root": "0202020202020202020202020202020202020202020202020202020202020202",
            "logs_root": "0303030303030303030303030303030303030303030303030303030303030303"
        }"#;
        let params = GenesisHeaderParams::from_json(json).expect("parse failed");

        assert_eq!(params.timestamp, 42);
        assert_eq!(params.epoch, 7);
        assert_eq!(params.parent_blkid, Buf32::from([0x01; 32]));
        assert_eq!(params.body_root, Buf32::from([0x02; 32]));
        assert_eq!(params.logs_root, Buf32::from([0x03; 32]));
    }

    #[test]
    fn test_header_partial_defaults() {
        // Only provide timestamp; everything else defaults.
        let json = r#"{ "timestamp": 100 }"#;
        let params = GenesisHeaderParams::from_json(json).expect("parse failed");

        assert_eq!(params.timestamp, 100);
        assert_eq!(params.epoch, 0);
        assert_eq!(params.parent_blkid, Buf32::zero());
        assert_eq!(params.body_root, Buf32::zero());
        assert_eq!(params.logs_root, Buf32::zero());
    }

    #[test]
    fn test_header_json_roundtrip() {
        let json = r#"{
            "timestamp": 10,
            "epoch": 3,
            "parent_blkid": "abababababababababababababababababababababababababababababababab",
            "body_root": "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            "logs_root": "efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef"
        }"#;
        let params = GenesisHeaderParams::from_json(json).expect("parse failed");
        let serialized = params.to_json().expect("serialization failed");
        let decoded = GenesisHeaderParams::from_json(&serialized).expect("deserialization failed");

        assert_eq!(params.timestamp, decoded.timestamp);
        assert_eq!(params.epoch, decoded.epoch);
        assert_eq!(params.parent_blkid, decoded.parent_blkid);
        assert_eq!(params.body_root, decoded.body_root);
        assert_eq!(params.logs_root, decoded.logs_root);
    }
}
