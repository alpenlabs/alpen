//! OL parameters.
//!
//! Provides JSON-serializable configuration for OL genesis state, including
//! genesis block header parameters, genesis account definitions, and the
//! initial L1 block commitment, plus runtime parameters that affect OL STF
//! execution.
mod account;
mod header;

use std::collections::BTreeMap;

pub use account::GenesisSnarkAccountData;
#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
pub use header::GenesisHeaderParams;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
pub use strata_bridge_params::BridgeParams;
use strata_identifiers::{AccountId, EpochCommitment, L1BlockCommitment};

/// OL genesis parameters.
///
/// These fields are needed to construct genesis state and do not need to be
/// embedded into proof programs after genesis initialization.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLGenesisParams {
    /// Header parameters for the parent of the genesis block.
    #[serde(default)]
    pub header: GenesisHeaderParams,

    /// Genesis accounts keyed by account ID.
    #[serde(default)]
    pub accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,

    /// Last L1 block known at genesis time, treated as the initial verified L1 tip.
    #[serde(default)]
    pub last_l1_block: L1BlockCommitment,
}

/// OL runtime parameters.
///
/// These fields affect OL STF execution and therefore must be bound to proof
/// artifacts when the STF runs inside a zkVM guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLRuntimeParams {
    /// Withdrawal denomination and optional cap.
    pub bridge_params: BridgeParams,
}

impl OLRuntimeParams {
    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }
}

/// Top-level OL params file.
///
/// This type separates genesis-only inputs from runtime parameters that are
/// needed when executing the OL STF.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLParams {
    /// Params used to construct OL genesis state.
    pub genesis: OLGenesisParams,

    /// Params used while executing the OL STF.
    pub runtime: OLRuntimeParams,
}

impl OLParams {
    /// Creates an [`OLParams`] from split genesis and runtime params.
    pub fn from_parts(genesis: OLGenesisParams, runtime: OLRuntimeParams) -> Self {
        Self { genesis, runtime }
    }

    /// Creates an [`OLParams`] with empty accounts and default header params.
    pub fn new_empty(last_l1_block: L1BlockCommitment, bridge_params: BridgeParams) -> Self {
        Self::from_parts(
            OLGenesisParams {
                header: GenesisHeaderParams::default(),
                accounts: BTreeMap::new(),
                last_l1_block,
            },
            OLRuntimeParams { bridge_params },
        )
    }

    /// Extracts the genesis-only portion of these params.
    pub fn genesis_params(&self) -> &OLGenesisParams {
        &self.genesis
    }

    /// Extracts the runtime portion of these params.
    pub fn runtime_params(&self) -> OLRuntimeParams {
        self.runtime
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        self.runtime.bridge_params()
    }

    /// Inserts an account into the OL genesis account set.
    pub fn insert_genesis_account(
        &mut self,
        account_id: AccountId,
        account: GenesisSnarkAccountData,
    ) -> Option<GenesisSnarkAccountData> {
        self.genesis.accounts.insert(account_id, account)
    }

    /// Returns the L1 block commitment used as OL genesis anchor.
    pub fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.genesis.last_l1_block
    }

    /// Builds an [`EpochCommitment`] from the genesis header parameters.
    ///
    /// The genesis header's epoch, slot, and parent block ID are treated as a
    /// checkpointed epoch, serving as the initial verified commitment.
    pub fn checkpointed_epoch(&self) -> EpochCommitment {
        EpochCommitment::new(
            self.genesis.header.epoch,
            self.genesis.header.slot,
            self.genesis.header.parent_blkid,
        )
    }
}

#[cfg(any(test, feature = "test-defaults"))]
#[expect(
    clippy::derivable_impls,
    reason = "OLParams defaults are only available in test builds and depend on gated bridge params defaults"
)]
impl Default for OLParams {
    fn default() -> Self {
        Self::from_parts(
            OLGenesisParams::default(),
            OLRuntimeParams {
                bridge_params: BridgeParams::default(),
            },
        )
    }
}

#[cfg(any(test, feature = "test-defaults"))]
impl Default for OLRuntimeParams {
    fn default() -> Self {
        Self {
            bridge_params: BridgeParams::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
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
            GenesisSnarkAccountData {
                predicate: PredicateKey::always_accept(),
                inner_state: Buf32::zero(),
                balance: BitcoinAmount::try_from(1000)
                    .expect("amount must not exceed the Bitcoin money supply"),
            },
        );

        accounts.insert(
            id2,
            GenesisSnarkAccountData {
                predicate: PredicateKey::always_accept(),
                inner_state: Buf32::from([0xab; 32]),
                balance: BitcoinAmount::default(),
            },
        );

        OLParams {
            genesis: OLGenesisParams {
                header: serde_json::from_str("{}").unwrap(),
                accounts,
                last_l1_block: L1BlockCommitment::default(),
            },
            runtime: OLRuntimeParams::default(),
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let params = sample_params();
        let json = serde_json::to_string(&params).expect("serialization failed");
        let decoded: OLParams = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(
            params.genesis.accounts.len(),
            decoded.genesis.accounts.len()
        );
        for (id, original) in &params.genesis.accounts {
            let restored = decoded.genesis.accounts.get(id).expect("missing account");
            assert_eq!(original.balance, restored.balance);
            assert_eq!(original.inner_state, restored.inner_state);
        }
    }

    #[test]
    fn test_split_params_accessors() {
        let params = sample_params();

        let genesis = params.genesis_params();
        assert_eq!(genesis.header.epoch, params.genesis.header.epoch);
        assert_eq!(genesis.header.slot, params.genesis.header.slot);
        assert_eq!(
            genesis.header.parent_blkid,
            params.genesis.header.parent_blkid
        );
        assert_eq!(genesis.accounts.len(), params.genesis.accounts.len());
        assert_eq!(genesis.last_l1_block, params.genesis.last_l1_block);

        let runtime = params.runtime_params();
        assert_eq!(runtime, params.runtime);
    }

    #[test]
    fn test_from_split_params_uses_nested_json_shape() {
        let params = sample_params();
        let rebuilt =
            OLParams::from_parts(params.genesis_params().clone(), params.runtime_params());
        let json = serde_json::to_value(&rebuilt).expect("serialization failed");

        assert!(json.get("genesis").is_some());
        assert!(json.get("runtime").is_some());
        assert!(json.get("header").is_none());
        assert!(json.get("accounts").is_none());
        assert!(json.get("last_l1_block").is_none());
        assert!(json.get("bridge_params").is_none());
    }

    #[test]
    fn test_runtime_params_ssz_roundtrip() {
        let params = sample_params().runtime_params();
        let encoded = params.as_ssz_bytes();
        let decoded = OLRuntimeParams::from_ssz_bytes(&encoded).expect("decode runtime params");

        assert_eq!(decoded.bridge_params, params.bridge_params);
    }

    #[test]
    fn test_balance_defaults_to_zero() {
        let json = r#"{
            "genesis": {
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
            },
            "runtime": {
                "bridge_params": {
                    "denomination": 100000000,
                    "max_withdrawal_amount": 1000000000,
                    "max_withdrawal_descriptor_len": 81
                }
            }
        }"#;

        let params = serde_json::from_str::<OLParams>(json).expect("parse failed");
        assert_eq!(params.genesis.accounts.len(), 2);

        let id1 = AccountId::from([1u8; 32]);
        let id2 = AccountId::from([2u8; 32]);

        assert_eq!(
            params.genesis.accounts[&id1].balance,
            BitcoinAmount::try_from(500).expect("amount must not exceed the Bitcoin money supply")
        );
        assert_eq!(
            params.genesis.accounts[&id2].balance,
            BitcoinAmount::default()
        );
    }

    #[test]
    fn test_empty_accounts_map() {
        let json = r#"{
            "genesis": {
                "header": {},
                "accounts": {},
                "last_l1_block": {
                    "height": 0,
                    "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
                }
            },
            "runtime": {
                "bridge_params": {
                    "denomination": 100000000,
                    "max_withdrawal_amount": 1000000000,
                    "max_withdrawal_descriptor_len": 81
                }
            }
        }"#;
        let params = serde_json::from_str::<OLParams>(json).expect("parse failed");
        assert!(params.genesis.accounts.is_empty());
    }

    #[test]
    fn test_missing_required_field_errors() {
        // Missing inner_state.
        let json = r#"{
            "genesis": {
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
            },
            "runtime": {
                "bridge_params": {
                    "denomination": 100000000,
                    "max_withdrawal_amount": 1000000000,
                    "max_withdrawal_descriptor_len": 81
                }
            }
        }"#;

        let result = serde_json::from_str::<OLParams>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_runtime_params_errors() {
        let json = r#"{
            "genesis": {
                "header": {},
                "accounts": {},
                "last_l1_block": {
                    "height": 0,
                    "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
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
        assert_eq!(
            params.genesis.accounts.len(),
            decoded.genesis.accounts.len()
        );
    }

    #[test]
    fn test_accounts_sorted_by_id() {
        let params = sample_params();
        let ids: Vec<_> = params.genesis.accounts.keys().collect();
        for window in ids.windows(2) {
            assert!(window[0] < window[1], "accounts should be sorted by ID");
        }
    }
}
