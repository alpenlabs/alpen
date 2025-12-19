use bitcoin::Block;
use strata_identifiers::{Epoch, L1BlockCommitment};

/// L1 events that we observe and want the persistence task to work on.
#[derive(Clone, Debug)]
pub(crate) enum L1Event {
    /// Data that contains block number, block and relevant transactions, and also the epoch whose
    /// rules are applied to.
    BlockData(BlockData, Epoch),

    /// Revert to the provided block height
    RevertTo(L1BlockCommitment),
}

/// Stores the bitcoin block and interpretations of relevant transactions within
/// the block.
#[derive(Clone, Debug)]
pub(crate) struct BlockData {
    /// Block number.
    block_num: u64,

    /// Raw block data.
    block: Block,
}

impl BlockData {
    pub(crate) fn new(block_num: u64, block: Block) -> Self {
        Self { block_num, block }
    }

    pub(crate) fn block_num(&self) -> u64 {
        self.block_num
    }

    pub(crate) fn block(&self) -> &Block {
        &self.block
    }
}
