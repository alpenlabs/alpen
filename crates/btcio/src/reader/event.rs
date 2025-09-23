use bitcoin::Block;
use strata_primitives::l1::L1BlockCommitment;

/// L1 events that we observe and want the persistence task to work on.
#[derive(Clone, Debug)]
pub(crate) enum L1Event {
    /// Data that contains block number, block and relevant transactions, and also the epoch whose
    /// rules are applied to.
    BlockData(BlockData, u64),

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
    // TODO remove?
    block: Block,

    /// Transaction indexes in the block that contain SPS-50 tags
    tagged_tx_indices: Vec<u32>,
}

impl BlockData {
    pub(crate) fn new(block_num: u64, block: Block, tagged_tx_indices: Vec<u32>) -> Self {
        Self {
            block_num,
            block,
            tagged_tx_indices,
        }
    }

    pub(crate) fn block_num(&self) -> u64 {
        self.block_num
    }

    pub(crate) fn block(&self) -> &Block {
        &self.block
    }

    pub(crate) fn tagged_tx_indices(&self) -> &[u32] {
        &self.tagged_tx_indices
    }
}
