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
    header: GenesisHeaderParams,

    /// Genesis accounts keyed by account ID.
    #[serde(default)]
    accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,

    /// Last L1 block known at genesis time, treated as the initial verified L1 tip.
    #[serde(default)]
    last_l1_block: L1BlockCommitment,
}

impl OLGenesisParams {
    pub fn header(&self) -> &GenesisHeaderParams {
        &self.header
    }

    pub fn accounts(&self) -> &BTreeMap<AccountId, GenesisSnarkAccountData> {
        &self.accounts
    }

    pub fn last_l1_block(&self) -> L1BlockCommitment {
        self.last_l1_block
    }
}

/// OL runtime parameters.
///
/// These fields affect OL STF execution and therefore must be bound to proof
/// artifacts when the STF runs inside a zkVM guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLRuntimeParams {
    /// Withdrawal denomination and optional cap.
    bridge_params: BridgeParams,
}

impl OLRuntimeParams {
    pub fn new(bridge_params: BridgeParams) -> Self {
        Self { bridge_params }
    }

    #[cfg(any(test, feature = "test-defaults"))]
    pub fn test_default() -> Self {
        Self::new(BridgeParams::default())
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Computes the SHA-256 hash of the SSZ-encoded runtime params.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.as_ssz_bytes()).into()
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
    genesis: OLGenesisParams,

    /// Params used while executing the OL STF.
    runtime: OLRuntimeParams,
}

impl OLParams {
    /// Starts building [`OLParams`] with explicit runtime params.
    pub fn builder(runtime: OLRuntimeParams) -> OLParamsBuilder {
        OLParamsBuilder::new(runtime)
    }

    fn new(genesis: OLGenesisParams, runtime: OLRuntimeParams) -> Self {
        Self { genesis, runtime }
    }

    #[cfg(any(test, feature = "test-defaults"))]
    pub fn test_default() -> Self {
        Self::builder(OLRuntimeParams::test_default()).build()
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

/// Builder for assembling immutable [`OLParams`].
#[derive(Clone, Debug)]
pub struct OLParamsBuilder {
    genesis: OLGenesisParams,
    runtime: OLRuntimeParams,
}

impl OLParamsBuilder {
    pub fn new(runtime: OLRuntimeParams) -> Self {
        Self {
            genesis: OLGenesisParams::default(),
            runtime,
        }
    }

    pub fn genesis_header(mut self, header: GenesisHeaderParams) -> Self {
        self.genesis.header = header;
        self
    }

    pub fn genesis_l1_block(mut self, last_l1_block: L1BlockCommitment) -> Self {
        self.genesis.last_l1_block = last_l1_block;
        self
    }

    pub fn genesis_accounts(
        mut self,
        accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,
    ) -> Self {
        self.genesis.accounts = accounts;
        self
    }

    pub fn build(self) -> OLParams {
        OLParams::new(self.genesis, self.runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params() -> OLParams {
        OLParams::test_default()
    }

    #[test]
    fn split_params_use_nested_json_shape() {
        let params = sample_params();
        let json = serde_json::to_value(&params).expect("serialization failed");

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
