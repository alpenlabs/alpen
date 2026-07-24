//! Embedded EVM chain spec.

use std::sync::Arc;

use alloy_genesis::Genesis;
use alpen_chainspec::{ee_genesis_block_info, AlpenEeGenesisBlockInfo};
use reth_chainspec::ChainSpec;
use serde::{Deserialize, Serialize, Serializer};

/// The embedded EVM chain spec: genesis document plus derived reth chain spec.
///
/// The JSON form is the standard EVM genesis document (chain config plus
/// allocation), exactly what `--custom-chain` used to load as a separate
/// file. Decoding eagerly derives the reth [`ChainSpec`], so every consumer
/// reads the same value and no boot-time genesis cross-check is needed —
/// agreement is structural.
///
/// Validity of the document is reth's concern, not ours: decoding accepts
/// exactly what reth's `Genesis -> ChainSpec` conversion accepts and derives
/// the spec from it, rather than re-policing individual genesis fields (which
/// would only diverge from — and lag behind — reth's own semantics).
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "Genesis")]
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

impl From<Genesis> for EvmSpec {
    fn from(genesis: Genesis) -> Self {
        let chain_spec: Arc<ChainSpec> = Arc::new(genesis.clone().into());
        Self {
            genesis,
            chain_spec,
        }
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
    fn json_accepts_what_reth_accepts() {
        // Validity is deferred to reth's `Genesis -> ChainSpec` conversion, so
        // even a minimal document decodes and derives genesis info without
        // policing individual fields (or panicking on an absent block number).
        let spec: EvmSpec = serde_json::from_str("{}").expect("empty genesis is accepted");
        let _ = spec.genesis_info();
    }
}
