//! Embedded EVM chain spec.

use std::sync::Arc;

use alloy_genesis::Genesis;
use alpen_chainspec::{ee_genesis_block_info, AlpenEeGenesisBlockInfo};
use reth_chainspec::ChainSpec;
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;

/// The embedded EVM chain spec: genesis document plus derived reth chain spec.
///
/// The JSON form is the standard EVM genesis document (chain config plus
/// allocation), exactly what `--custom-chain` used to load as a separate
/// file. Decoding eagerly derives the reth [`ChainSpec`], so every consumer
/// reads the same value and no boot-time genesis cross-check is needed —
/// agreement is structural.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "Genesis")]
pub struct EvmSpec {
    /// The parsed genesis document, authoritative for serialization.
    genesis: Genesis,

    /// Chain spec derived from `genesis`; never serialized.
    ///
    /// Stored as `Arc<ChainSpec>` — not because [`EvmSpec`] needs shared
    /// ownership, but to match reth's boundary: the node's `command.chain`
    /// field is `Arc<ChainSpec>`, so consumers hand it off with a cheap
    /// refcount bump rather than deep-cloning the spec (or re-deriving it
    /// from `genesis`) on every use.
    chain_spec: Arc<ChainSpec>,
}

impl EvmSpec {
    /// Returns the genesis document.
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    /// Returns the derived reth chain spec.
    pub fn chain_spec(&self) -> &Arc<ChainSpec> {
        &self.chain_spec
    }

    /// Returns the genesis block facts, derived from the chain spec on demand.
    pub fn genesis_info(&self) -> AlpenEeGenesisBlockInfo {
        ee_genesis_block_info(&self.chain_spec)
    }
}

impl TryFrom<Genesis> for EvmSpec {
    type Error = EvmSpecError;

    fn try_from(genesis: Genesis) -> Result<Self, Self::Error> {
        // `Genesis` is struct-level `#[serde(default)]`, so an empty or
        // misshapen document would otherwise decode to all-default values;
        // these checks are the validate-on-decode backstop. A zero gas limit
        // (the default) marks a document with no real block parameters, and
        // an omitted chain id defaults to 1 so only an explicit zero can be
        // caught here.
        if genesis.number.is_none() {
            return Err(EvmSpecError::MissingGenesisNumber);
        }
        if genesis.gas_limit == 0 {
            return Err(EvmSpecError::ZeroGasLimit);
        }
        if genesis.config.chain_id == 0 {
            return Err(EvmSpecError::ZeroChainId);
        }

        let chain_spec: Arc<ChainSpec> = Arc::new(genesis.clone().into());

        Ok(Self {
            genesis,
            chain_spec,
        })
    }
}

impl Serialize for EvmSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.genesis.serialize(serializer)
    }
}

// Compare the genesis document only: `ChainSpec`'s derived equality goes
// through `SealedHeader`'s lazily initialized hash cache and is therefore
// initialization-order-sensitive, and everything else here is derived from
// `genesis` anyway.
impl PartialEq for EvmSpec {
    fn eq(&self, other: &Self) -> bool {
        self.genesis == other.genesis
    }
}

impl Eq for EvmSpec {}

/// Error validating an EVM genesis document.
#[derive(Debug, Error)]
pub enum EvmSpecError {
    /// The genesis document carries no block number.
    #[error("genesis block number missing from EVM genesis document")]
    MissingGenesisNumber,

    /// The genesis document carries no (or a zero) gas limit.
    #[error("gas limit missing or zero in EVM genesis document")]
    ZeroGasLimit,

    /// The genesis document carries an explicit zero chain id.
    #[error("chain id is zero in EVM genesis document")]
    ZeroChainId,
}

#[cfg(test)]
mod tests {
    use alpen_chainspec::{ee_genesis_block_info_from_json, DEV_CHAIN_SPEC};

    use super::EvmSpec;

    #[test]
    fn json_roundtrip_preserves_evm_spec() {
        let spec: EvmSpec = serde_json::from_str(DEV_CHAIN_SPEC).expect("dev chain should parse");

        let json = serde_json::to_string(&spec).expect("evm spec should serialize");
        let decoded: EvmSpec = serde_json::from_str(&json).expect("evm spec should reparse");

        assert_eq!(decoded, spec);
        assert_eq!(decoded.genesis_info(), spec.genesis_info());
    }

    #[test]
    fn genesis_info_matches_chainspec_derivation() {
        let spec: EvmSpec = serde_json::from_str(DEV_CHAIN_SPEC).expect("dev chain should parse");
        let expected =
            ee_genesis_block_info_from_json(DEV_CHAIN_SPEC).expect("dev chain should parse");

        assert_eq!(spec.genesis_info(), expected);
        assert_eq!(spec.chain_spec().genesis_hash(), expected.blockhash());
    }

    #[test]
    fn json_rejects_degenerate_genesis() {
        // `Genesis` is `#[serde(default)]`; these must not silently decode.
        assert!(serde_json::from_str::<EvmSpec>("{}").is_err());
        // Missing block number.
        assert!(serde_json::from_str::<EvmSpec>(r#"{"config":{"chainId":2892}}"#).is_err());
        // Missing gas limit.
        assert!(serde_json::from_str::<EvmSpec>(r#"{"number":"0x0"}"#).is_err());
        // Explicit zero chain id.
        assert!(serde_json::from_str::<EvmSpec>(
            r#"{"number":"0x0","gasLimit":"0x1c9c380","config":{"chainId":0}}"#
        )
        .is_err());
    }
}
