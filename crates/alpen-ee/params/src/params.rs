//! Top-level Alpen params artifact.

use std::sync::Arc;

use alpen_chainspec::AlpenEeGenesisBlockInfo;
use reth_chainspec::ChainSpec;
use serde::{Deserialize, Serialize};
use strata_acct_types::AccountId;
use strata_bridge_params::BridgeParams;

use crate::{AlpenSpecActivations, BlobSpec, EvmSpec};

/// Default Alpen EE account id registered in generated OL params.
pub const DEFAULT_ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

/// Top-level Alpen chain params.
///
/// The single source of truth for how a node interprets the chain: the EE
/// account identity, bridge economics, DA stream identity, the Alpen spec
/// activations, and the embedded EVM chain spec. Loaded from one JSON artifact
/// with validate-on-decode semantics on every field.
///
/// Unknown fields are rejected so that a params file written for a newer
/// node version (e.g. one carrying spec activations this binary does not
/// understand) fails loudly instead of being silently misread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlpenParams {
    /// Account id of the EE in OL. Fork-invariant.
    account_id: AccountId,

    /// Bridge denomination and withdrawal policy.
    bridge_params: BridgeParams,

    /// DA stream identity.
    blob_spec: BlobSpec,

    /// Alpen protocol spec activations.
    #[serde(default)]
    spec_activations: AlpenSpecActivations,

    /// Embedded EVM chain spec (genesis document + fork configuration).
    evm_spec: EvmSpec,
}

impl AlpenParams {
    /// Creates new chain params.
    pub fn new(
        account_id: AccountId,
        bridge_params: BridgeParams,
        blob_spec: BlobSpec,
        spec_activations: AlpenSpecActivations,
        evm_spec: EvmSpec,
    ) -> Self {
        Self {
            account_id,
            bridge_params,
            blob_spec,
            spec_activations,
            evm_spec,
        }
    }

    /// Parses chain params from a JSON string.
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serializes chain params to pretty-printed JSON.
    pub fn to_json_string_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Returns the EE account ID in the OL chain.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns the bridge denomination and withdrawal policy.
    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Returns the DA stream identity.
    pub fn blob_spec(&self) -> BlobSpec {
        self.blob_spec
    }

    /// Returns the Alpen spec activations.
    pub fn spec_activations(&self) -> &AlpenSpecActivations {
        &self.spec_activations
    }

    /// Returns the embedded EVM chain spec.
    pub fn evm_spec(&self) -> &EvmSpec {
        &self.evm_spec
    }

    /// Returns the derived reth chain spec.
    pub fn chain_spec(&self) -> &Arc<ChainSpec> {
        self.evm_spec.chain_spec()
    }

    /// Returns the derived execution genesis block facts.
    pub fn genesis_block_info(&self) -> AlpenEeGenesisBlockInfo {
        self.evm_spec.genesis_info()
    }
}

#[cfg(test)]
mod tests {
    use alpen_chainspec::DEV_CHAIN_SPEC;
    use serde_json::{json, Value};
    use strata_bridge_params::BridgeParams;
    use strata_l1_txfmt::MagicBytes;

    use super::{AlpenParams, DEFAULT_ALPEN_EE_ACCOUNT_ID};
    use crate::{AlpenSpecActivations, BlobSpec, EvmSpec};

    fn sample_params() -> AlpenParams {
        let evm_spec: EvmSpec =
            serde_json::from_str(DEV_CHAIN_SPEC).expect("dev chain should parse");
        AlpenParams::new(
            DEFAULT_ALPEN_EE_ACCOUNT_ID,
            BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 81)
                .expect("valid bridge params"),
            BlobSpec::new(MagicBytes::new(*b"ALPN")),
            AlpenSpecActivations::default(),
            evm_spec,
        )
    }

    fn sample_json() -> Value {
        serde_json::to_value(sample_params()).expect("params should serialize")
    }

    #[test]
    fn json_roundtrip_preserves_params() {
        let params = sample_params();

        let json = params
            .to_json_string_pretty()
            .expect("params should serialize");
        let decoded = AlpenParams::from_json_str(&json).expect("params should deserialize");

        assert_eq!(decoded, params);
    }

    #[test]
    fn json_defaults_missing_spec_activations_to_empty() {
        let mut json = sample_json();
        json.as_object_mut()
            .expect("params should be an object")
            .remove("spec_activations")
            .expect("spec_activations should be present");

        let decoded: AlpenParams = serde_json::from_value(json).expect("params should deserialize");
        assert!(decoded.spec_activations().is_empty());
    }

    #[test]
    fn json_rejects_missing_bridge_params() {
        let mut json = sample_json();
        json.as_object_mut()
            .expect("params should be an object")
            .remove("bridge_params")
            .expect("bridge_params should be present");

        assert!(serde_json::from_value::<AlpenParams>(json).is_err());
    }

    #[test]
    fn json_rejects_unknown_fields() {
        let mut json = sample_json();
        json.as_object_mut()
            .expect("params should be an object")
            .insert("genesis_blockhash".to_owned(), json!("0xdeadbeef"));

        assert!(serde_json::from_value::<AlpenParams>(json).is_err());
    }

    #[test]
    fn json_rejects_malformed_account_id() {
        let mut json = sample_json();
        json.as_object_mut()
            .expect("params should be an object")
            .insert("account_id".to_owned(), json!("01"));

        assert!(serde_json::from_value::<AlpenParams>(json).is_err());
    }
}
