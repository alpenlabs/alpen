use alloy_genesis::Genesis;
use alloy_primitives::FixedBytes;

use crate::types::AccountId;

/// Chain specific config, that needs to remain constant on all nodes
/// to ensure all stay on the same chain.
#[derive(Debug, Clone)]
pub struct Params {
    /// Account id of current EE in OL
    pub account_id: AccountId,

    /// Genesis config defining the current evm chain
    pub genesis_config: Genesis,
}

/// Local config that may differ between nodes.
#[derive(Debug, Clone)]
pub struct Config {
    /// Chain specific config
    pub params: Params,

    /// number of blocks behind L1 tip block to consider safe from reorg
    pub finality_depth: u64,

    /// Used to verify head updates from sequencer over p2p
    pub sequencer_pubkey: FixedBytes<32>,
}

/// Sequencer specific config, for block and batch assembly
#[derive(Debug, Clone)]
pub struct SequencerConfig {
    /// target blocktime for block production, in milliseconds
    pub target_blocktime_ms: u32,

    /// number of blocks behind L1 tip block, from which to include data in block production.
    /// This defined the L1/OL block depth that the sequencer is anchoring its own EE blocks to,
    /// for reorg resistance. As long as L1 does not reorg to this depth, EE can ignore
    ///
    /// `config.finality_depth >= anchor_finality_depth`
    pub anchor_finality_depth: u32,
}
