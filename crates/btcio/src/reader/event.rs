use bitcoin::Block;
use strata_l1tx::messages::RelevantTxEntry;
use strata_primitives::l1::L1BlockCommitment;

/// L1 events that we observe and want the persistence task to work on.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
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

    /// Transactions in the block that contain protocol operations
    relevant_txs: Vec<RelevantTxEntry>,
}

impl BlockData {
    pub(crate) fn new(block_num: u64, block: Block, relevant_txs: Vec<RelevantTxEntry>) -> Self {
        Self {
            block_num,
            block,
            relevant_txs,
        }
    }

    pub(crate) fn block_num(&self) -> u64 {
        self.block_num
    }

    pub(crate) fn block(&self) -> &Block {
        &self.block
    }

    pub(crate) fn relevant_txs(&self) -> &[RelevantTxEntry] {
        &self.relevant_txs
    }
}
