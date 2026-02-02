//! OL genesis parameters.
//!
//! Provides JSON-serializable configuration for OL genesis state, including
//! genesis block header parameters, genesis account definitions, and the
//! initial L1 block commitment.
mod account;
mod header;

use std::collections::BTreeMap;

pub use account::AccountParams;
#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
pub use header::HeaderParams;
use serde::{Deserialize, Serialize};
use strata_identifiers::{AccountId, EpochCommitment, L1BlockCommitment};

/// Top-level OL genesis parameters.
///
/// Combines header parameters and genesis account definitions into a single
/// configuration structure.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLParams {
    /// Header parameters for the parent of the genesis block.
    pub header: HeaderParams,

    /// Genesis accounts keyed by account ID.
    pub accounts: BTreeMap<AccountId, AccountParams>,

    /// Last L1 block known at genesis time, treated as the initial verified L1 tip.
    pub last_l1_block: L1BlockCommitment,
}

impl OLParams {
    /// Builds an [`EpochCommitment`] from the genesis header parameters.
    ///
    /// The genesis header's epoch, slot, and parent block ID are treated as a
    /// checkpointed epoch, serving as the initial verified commitment.
    pub fn checkpointed_epoch(&self) -> EpochCommitment {
        EpochCommitment::new(
            self.header.epoch,
            self.header.slot,
            self.header.parent_blkid,
        )
    }
}

#[cfg(test)]
mod tests {
    use strata_btc_types::BitcoinAmount;
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
                balance: BitcoinAmount::from_sat(1000),
            },
        );

        accounts.insert(
            id2,
            AccountParams {
                predicate: PredicateKey::always_accept(),
                inner_state: Buf32::from([0xab; 32]),
                balance: BitcoinAmount::ZERO,
            },
        );

        OLParams {
            header: serde_json::from_str("{}").unwrap(),
            accounts,
            last_l1_block: L1BlockCommitment::default(),
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
            },
            "last_l1_block": {
                "height": 0,
                "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
            }
        }"#;

        let params = serde_json::from_str::<OLParams>(json).expect("parse failed");
        assert_eq!(params.accounts.len(), 2);

        let id1 = AccountId::from([1u8; 32]);
        let id2 = AccountId::from([2u8; 32]);

        assert_eq!(params.accounts[&id1].balance, BitcoinAmount::from_sat(500));
        assert_eq!(params.accounts[&id2].balance, BitcoinAmount::ZERO);
    }

    #[test]
    fn test_empty_accounts_map() {
        let json = r#"{
            "header": {},
            "accounts": {},
            "last_l1_block": {
                "height": 0,
                "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
            }
        }"#;
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
            },
            "last_l1_block": {
                "height": 0,
                "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
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
