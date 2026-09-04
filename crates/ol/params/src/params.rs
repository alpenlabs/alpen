use std::collections::BTreeMap;

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ssz::Encode;
use ssz_derive::{Decode, Encode};
use strata_identifiers::{AccountId, EpochCommitment, L1BlockCommitment};

use crate::{BridgeParams, GenesisHeaderParams, GenesisSnarkAccountData};

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
#[cfg_attr(any(test, feature = "test-defaults"), derive(Default))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLRuntimeParams {
    /// Withdrawal denomination and optional cap.
    bridge_params: BridgeParams,
}

impl OLRuntimeParams {
    pub fn new(bridge_params: BridgeParams) -> Self {
        Self { bridge_params }
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Computes the SHA-256 hash of the SSZ-encoded runtime params.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.as_ssz_bytes()).into()
    }

    /// Computes the hex-encoded hash of the runtime params.
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash())
    }
}

/// Top-level OL params file.
///
/// This type separates genesis-only inputs from runtime parameters that are
/// needed when executing the OL STF.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "test-defaults"), derive(Default))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLParams {
    /// Params used to construct OL genesis state.
    genesis: OLGenesisParams,

    /// Params used while executing the OL STF.
    runtime: OLRuntimeParams,
}

impl OLParams {
    /// Creates an [`OLParams`] from split genesis and runtime params.
    pub fn new(genesis: OLGenesisParams, runtime: OLRuntimeParams) -> Self {
        Self { genesis, runtime }
    }

    /// Creates an [`OLParams`] with empty accounts and default header params.
    pub fn new_empty(last_l1_block: L1BlockCommitment, bridge_params: BridgeParams) -> Self {
        Self::new(
            OLGenesisParams {
                header: GenesisHeaderParams::default(),
                accounts: BTreeMap::new(),
                last_l1_block,
            },
            OLRuntimeParams::new(bridge_params),
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
    pub fn derive_genesis_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::new(
            self.genesis.header.epoch,
            self.genesis.header.slot,
            self.genesis.header.parent_blkid,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params() -> OLParams {
        OLParams::new(OLGenesisParams::default(), OLRuntimeParams::default())
    }

    #[test]
    fn split_params_use_nested_json_shape() {
        let params = sample_params();
        let rebuilt = OLParams::new(params.genesis_params().clone(), params.runtime_params());
        let json = serde_json::to_value(&rebuilt).expect("serialization failed");

        assert!(json.get("genesis").is_some());
        assert!(json.get("runtime").is_some());
        assert!(json.get("header").is_none());
        assert!(json.get("accounts").is_none());
        assert!(json.get("last_l1_block").is_none());
        assert!(json.get("bridge_params").is_none());
    }

    #[test]
    fn missing_runtime_params_errors() {
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
}
