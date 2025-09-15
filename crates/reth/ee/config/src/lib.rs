use alloy_genesis::Genesis;
use alpen_ee_primitives::AccountAddress;
use strata_primitives::{block_credential::CredRule, buf::Buf32};

/// Configurations that MUST remain same across all nodes in network.
/// Nodes with different `ChainParams` may diverge in their view of the chain.
#[derive(Debug, Clone)]
pub struct ChainParams {
    pub account: AccountAddress,
    pub genesis_config: Genesis,
}

/// Configurations for local operation of the alpen-ee-client.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// host:port of ol node to connect to. This OL is trusted by the client and should ideally be
    /// run by the same entity.
    pub ol_connection: String,
    /// pubkey for sequencer/block builder to verify authenticity of unproven updates received from
    /// network
    pub sequencer_credrule: CredRule,
    /// number of L1 blocks behind tip, whose corresponding ee block is marked as "safe" in rpc.
    pub safe_depth_l1: u64,
    /// number of L1 blocks behind tip, whose corresponding ee block is marked as "finalized" in
    /// rpc. finality_depth_l1 >= safe_depth_l1
    pub finality_depth_l1: u64,
}

/// Configurations specific to the sequencer node.
#[derive(Debug, Clone)]
pub struct SequencerConfig {
    /// private key for sequencer to sign block updates to p2p.
    pub sequencer_identity: Identity,
    /// blocktime that the sequencer should target to maintain, in millis.
    pub target_blocktime_ms: u64,
    /// number of L1 blocks behind tip, to include OL updates from during block/batch production
    /// for reorg safety.
    pub anchor_depth_l1: u64,
    /// conditions where a batch should be sealed, TBD.
    pub batching_criterion: BatchingCriterion,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BatchingCriterion {
    /// Create a batch every fixed number of blocks
    FixedBlockCount(u64),
}

/// Private key for sequencer's signatures
pub type Identity = Buf32;
