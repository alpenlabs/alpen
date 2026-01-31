//! OL genesis account parameters.
//!
//! Provides JSON-serializable configuration for genesis OL accounts. At genesis
//! construction time, each account entry is used to build an [`OLAccountState`]
//! with auto-assigned serials starting at 128 (`SYSTEM_RESERVED_ACCTS`).
//!
//! Currently only **snark accounts** are supported. Empty accounts are not
//! supported in genesis configuration.
//!
//! [`OLAccountState`]: https://docs.rs/strata-ol-state-types

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_identifiers::{AccountId, Buf32};
use strata_predicate::PredicateKey;

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
}
