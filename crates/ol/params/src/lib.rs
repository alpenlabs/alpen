//! OL genesis parameters.
//!
//! Provides JSON-serializable configuration for OL genesis state, including
//! genesis block header parameters and genesis account definitions.
//!
//! ## Header parameters
//!
//! [`HeaderParams`] configures the genesis block header. All fields
//! default to zero values when omitted.
//!
//! ## Account parameters
//!
//! [`AccountParams`] configures individual genesis OL accounts. At genesis
//! construction time, each account entry is used to build an [`OLAccountState`]
//! with auto-assigned serials starting at 128 (`SYSTEM_RESERVED_ACCTS`).
//!
//! Currently only **snark accounts** are supported. Empty accounts are not
//! supported in genesis configuration.
//!
//! [`OLAccountState`]: https://docs.rs/strata-ol-state-types

mod account;
mod header;

pub use account::AccountParams;
pub use header::HeaderParams;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_identifiers::AccountId;

/// Top-level OL genesis parameters.
///
/// Combines header parameters and genesis account definitions into a single
/// configuration structure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OLParams {
    /// Genesis block header parameters.
    pub header: HeaderParams,

    /// Genesis accounts keyed by account ID.
    pub accounts: BTreeMap<AccountId, AccountParams>,
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;
    use strata_predicate::PredicateKey;

    use super::*;

    fn sample_params() -> OLParams {
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

        OLParams {
            header: serde_json::from_str("{}").unwrap(),
            accounts,
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let params = sample_params();
        let json = serde_json::to_string(&params).expect("serialization failed");
        let decoded: OLParams = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(params.accounts.len(), decoded.accounts.len());
        for (id, original) in &params.accounts {
            let restored = decoded.accounts.get(id).expect("missing account");
            assert_eq!(original.balance, restored.balance);
            assert_eq!(original.inner_state, restored.inner_state);
        }
    }

    #[test]
    fn test_balance_defaults_to_zero() {
        let json = r#"{
            "header": {},
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

        let params = serde_json::from_str::<OLParams>(json).expect("parse failed");
        assert_eq!(params.accounts.len(), 2);

        let id1 = AccountId::from([1u8; 32]);
        let id2 = AccountId::from([2u8; 32]);

        assert_eq!(params.accounts[&id1].balance, 500);
        assert_eq!(params.accounts[&id2].balance, 0);
    }

    #[test]
    fn test_empty_accounts_map() {
        let json = r#"{ "header": {}, "accounts": {} }"#;
        let params = serde_json::from_str::<OLParams>(json).expect("parse failed");
        assert!(params.accounts.is_empty());
    }

    #[test]
    fn test_missing_required_field_errors() {
        // Missing inner_state.
        let json = r#"{
            "header": {},
            "accounts": {
                "0101010101010101010101010101010101010101010101010101010101010101": {
                    "predicate": "AlwaysAccept"
                }
            }
        }"#;

        let result = serde_json::from_str::<OLParams>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_pretty_json_output() {
        let params = sample_params();
        let pretty = serde_json::to_string_pretty(&params).expect("pretty serialization failed");
        assert!(pretty.contains('\n'));
        let decoded: OLParams = serde_json::from_str(&pretty).expect("deserialization failed");
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
