//! Chunk builder state.

use std::{collections::VecDeque, mem};

use alpen_ee_common::{BatchId, BlockNumHash, ChunkId, ChunkStorage};
use eyre::Result;
use tracing::debug;

use crate::policy::{AccumulationPolicy, Accumulator};

/// An entry in the chunk builder's processing queue.
#[derive(Debug, Clone)]
pub enum PendingEntry {
    /// Seal the current chunk at the batch boundary and
    /// link chunks to the batch. No data dependency — processed immediately.
    BatchBoundary(BatchId),
    /// A block to accumulate. Carries the batch index from the batch
    /// builder event. Requires block data from the provider; deferred
    /// if data is not yet available.
    Block { block: BlockNumHash, batch_idx: u64 },
}

/// Mutable state for the chunk builder task.
#[expect(
    missing_debug_implementations,
    reason = "Accumulator<P> does not impl Debug uniformly"
)]
pub struct ChunkBuilderState<P: AccumulationPolicy> {
    /// Last block of the most recently sealed chunk (or genesis).
    prev_chunk_end: BlockNumHash,
    /// Index for the next chunk to be created.
    next_chunk_idx: u64,
    /// Accumulator for the current pending chunk.
    accumulator: Accumulator<P>,
    /// Batch index the chunk builder is currently building chunks for.
    /// Updated when a batch boundary is sealed.
    current_batch_idx: u64,
    /// Chunk IDs sealed for the current (not-yet-sealed) batch.
    /// Cleared when chunks are linked to their batch via `set_batch_chunks`
    /// is called.
    current_batch_chunks: Vec<ChunkId>,
    /// Processing queue: batch boundaries and blocks.
    pending: VecDeque<PendingEntry>,
}

impl<P: AccumulationPolicy> ChunkBuilderState<P> {
    /// Create state from the last sealed chunk.
    pub fn from_last_chunk(chunk_idx: u64, last_block: BlockNumHash, batch_idx: u64) -> Self {
        Self {
            prev_chunk_end: last_block,
            next_chunk_idx: chunk_idx + 1,
            accumulator: Accumulator::new(),
            current_batch_idx: batch_idx,
            current_batch_chunks: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    /// Create initial state when no chunks exist yet.
    pub fn new(genesis: BlockNumHash) -> Self {
        Self {
            prev_chunk_end: genesis,
            next_chunk_idx: 0,
            accumulator: Accumulator::new(),
            current_batch_idx: 0,
            current_batch_chunks: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    pub fn prev_chunk_end(&self) -> BlockNumHash {
        self.prev_chunk_end
    }

    /// The blocknum of the last known block — either the last block
    /// in the accumulator, or `prev_chunk_end` if the accumulator is
    /// empty. Used for overlap/gap detection on incoming events.
    pub fn last_known_blocknum(&self) -> u64 {
        self.accumulator
            .last_block()
            .map(|b| b.blocknum())
            .unwrap_or(self.prev_chunk_end.blocknum())
    }

    pub fn next_chunk_idx(&self) -> u64 {
        self.next_chunk_idx
    }

    pub fn accumulator(&self) -> &Accumulator<P> {
        &self.accumulator
    }

    pub fn accumulator_mut(&mut self) -> &mut Accumulator<P> {
        &mut self.accumulator
    }

    pub fn current_batch_idx(&self) -> u64 {
        self.current_batch_idx
    }

    pub fn set_current_batch_idx(&mut self, idx: u64) {
        self.current_batch_idx = idx;
    }

    pub fn current_batch_chunks(&self) -> &[ChunkId] {
        &self.current_batch_chunks
    }

    pub fn push_batch_chunk(&mut self, chunk_id: ChunkId) {
        self.current_batch_chunks.push(chunk_id);
    }

    pub fn take_batch_chunks(&mut self) -> Vec<ChunkId> {
        mem::take(&mut self.current_batch_chunks)
    }

    /// Advance state after sealing a chunk.
    pub fn advance_chunk(&mut self, last_block: BlockNumHash) {
        self.prev_chunk_end = last_block;
        self.next_chunk_idx += 1;
    }

    /// Override next chunk idx (used after reorg recovery from storage).
    pub fn set_next_chunk_idx(&mut self, idx: u64) {
        self.next_chunk_idx = idx;
    }

    /// Reset state after a reorg.
    pub fn reset_to(&mut self, revert_to: BlockNumHash) {
        self.prev_chunk_end = revert_to;
        self.accumulator.reset();
        self.current_batch_chunks.clear();
        self.pending.clear();
    }

    // -- Processing queue --

    pub fn push_pending(&mut self, entry: PendingEntry) {
        self.pending.push_back(entry);
    }

    pub fn peek_pending(&self) -> Option<&PendingEntry> {
        self.pending.front()
    }

    pub fn pop_pending(&mut self) -> Option<PendingEntry> {
        self.pending.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Initialize chunk builder state from storage.
///
/// Must be called **after** `cleanup_orphaned_chunks` and
/// `repair_batch_linkage`, which guarantee that every stored chunk
/// belongs to a linked batch at a complete boundary. The chunk builder
/// may still be behind the latest batch; `enqueue_backfill` handles
/// that gap.
pub async fn init_chunk_builder_state<P: AccumulationPolicy>(
    chunk_storage: &dyn ChunkStorage,
    genesis: BlockNumHash,
) -> Result<ChunkBuilderState<P>> {
    if let Some((chunk, _)) = chunk_storage.get_latest_chunk().await? {
        // After cleanup + repair, the latest chunk's batch is always
        // linked, so the next batch to build for is batch_idx + 1.
        let current_batch_idx = chunk.batch_idx() + 1;

        debug!(
            chunk_idx = chunk.idx(),
            last_block = %chunk.last_block(),
            current_batch_idx,
            "resuming chunk builder from storage"
        );
        Ok(ChunkBuilderState::from_last_chunk(
            chunk.idx(),
            BlockNumHash::new(chunk.last_block(), chunk.last_blocknum()),
            current_batch_idx,
        ))
    } else {
        // No chunks: genesis batch (idx=0) always exists, real batches start at 1.
        debug!("starting chunk builder from genesis");
        let mut state = ChunkBuilderState::new(genesis);
        state.set_current_batch_idx(1);
        Ok(state)
    }
}
