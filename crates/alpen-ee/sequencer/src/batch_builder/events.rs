//! Events emitted by the batch builder for downstream consumers.
//!
//! The chunk builder consumes these events via an mpsc channel to track
//! block processing and batch sealing without independently watching
//! `preconf_rx`. This guarantees the chunk builder never runs ahead of
//! the batch builder and inherits reorg handling for free.

use alpen_ee_common::{BatchId, BlockNumHash};

/// Event emitted by the batch builder after processing a block or
/// handling a reorg.
///
/// Sent on a bounded [`tokio::sync::mpsc`] channel. The chunk builder
/// is the sole consumer.
#[derive(Debug, Clone)]
pub enum BatchBuilderEvent {
    /// A block was processed and added to the batch accumulator.
    BlockProcessed(BlockProcessedEventData),
    /// A reorg was handled by the batch builder. The chunk builder
    /// must revert to match.
    Reorg(ReorgEventData),
}

impl BatchBuilderEvent {
    pub fn block_processed(
        block: BlockNumHash,
        batch_idx: u64,
        batch_sealed: Option<BatchId>,
    ) -> Self {
        Self::BlockProcessed(BlockProcessedEventData {
            block,
            batch_idx,
            batch_sealed,
        })
    }

    pub fn reorg(revert_to: BlockNumHash, last_valid_batch_idx: u64) -> Self {
        Self::Reorg(ReorgEventData {
            revert_to,
            last_valid_batch_idx,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BlockProcessedEventData {
    /// The block that was just accumulated. When `batch_sealed` is
    /// `Some`, this block is the **first block of the next batch**
    /// — it was added to the accumulator *after* the previous batch
    /// was sealed.
    pub block: BlockNumHash,
    /// Index of the batch this block belongs to. The chunk builder
    /// uses this to set `Chunk::batch_idx` and to validate that
    /// events arrive in the expected order.
    pub batch_idx: u64,
    /// Set when a batch was sealed immediately before this block
    /// was accumulated. The sealed batch contains the *previous*
    /// accumulator's blocks (not this one). The chunk builder must
    /// force-seal its current chunk at this boundary and call
    /// [`ChunkStorage::set_batch_chunks`](alpen_ee_common::ChunkStorage::set_batch_chunks).
    pub batch_sealed: Option<BatchId>,
}

#[derive(Debug, Clone)]
pub struct ReorgEventData {
    /// The new "last good" block. Corresponds to
    /// `state.prev_batch_end()` after the batch builder handled
    /// the reorg.
    pub revert_to: BlockNumHash,
    /// Index of the last canonical batch after the revert.
    pub last_valid_batch_idx: u64,
}
