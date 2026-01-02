//! State for the batch builder task.

use std::collections::VecDeque;

use alpen_ee_common::BatchStorage;
use eyre::Result;
use strata_acct_types::Hash;

use super::{Accumulator, BatchPolicy};

/// State for the batch builder task.
///
/// This tracks the current position in the chain and the accumulator for
/// the pending batch. This state is not persisted; it is rebuilt from
/// `BatchStorage` on restart.
#[derive(Debug)]
pub struct BatchBuilderState<P: BatchPolicy> {
    /// Hash of the last block in the most recent sealed batch (or genesis if no batches).
    prev_batch_end: Hash,
    /// Index for the next batch to be created.
    next_batch_idx: u64,
    /// Accumulator for the pending batch.
    accumulator: Accumulator<P>,
    /// Queue of block hashes waiting to be processed (data may not be ready yet).
    pending_blocks: VecDeque<Hash>,
}

impl<P: BatchPolicy> BatchBuilderState<P> {
    /// Initialize state from genesis (no batches exist yet).
    ///
    /// The first batch will have `prev_block` set to the genesis hash.
    pub fn from_genesis(genesis_hash: Hash) -> Self {
        Self {
            prev_batch_end: genesis_hash,
            next_batch_idx: 0,
            accumulator: Accumulator::new(),
            pending_blocks: VecDeque::new(),
        }
    }

    /// Initialize state from the last sealed batch.
    ///
    /// Used when resuming from storage where batches already exist.
    pub fn from_last_batch(batch_idx: u64, last_block: Hash) -> Self {
        Self {
            prev_batch_end: last_block,
            next_batch_idx: batch_idx + 1,
            accumulator: Accumulator::new(),
            pending_blocks: VecDeque::new(),
        }
    }

    /// Get the hash of the last block in the previous batch.
    pub fn prev_batch_end(&self) -> Hash {
        self.prev_batch_end
    }

    /// Get the index for the next batch to be created.
    pub fn next_batch_idx(&self) -> u64 {
        self.next_batch_idx
    }

    /// Get a reference to the accumulator.
    pub fn accumulator(&self) -> &Accumulator<P> {
        &self.accumulator
    }

    /// Get a mutable reference to the accumulator.
    pub fn accumulator_mut(&mut self) -> &mut Accumulator<P> {
        &mut self.accumulator
    }

    /// Called after sealing a batch.
    ///
    /// Advances the state to prepare for the next batch.
    pub fn advance_batch(&mut self, new_prev_batch_end: Hash) {
        self.prev_batch_end = new_prev_batch_end;
        self.next_batch_idx += 1;
        self.accumulator.reset();
    }

    /// Returns the first pending block hash, if any.
    pub fn first_pending_block(&self) -> Option<Hash> {
        self.pending_blocks.front().copied()
    }

    /// Returns true if there are pending blocks to process.
    pub fn has_pending_blocks(&self) -> bool {
        !self.pending_blocks.is_empty()
    }

    /// Removes and returns the first pending block.
    pub fn pop_pending_block(&mut self) -> Option<Hash> {
        self.pending_blocks.pop_front()
    }

    /// Adds blocks to the pending queue.
    pub fn push_pending_blocks(&mut self, blocks: impl IntoIterator<Item = Hash>) {
        self.pending_blocks.extend(blocks);
    }

    /// Clears the pending blocks queue.
    pub fn clear_pending_blocks(&mut self) {
        self.pending_blocks.clear();
    }

    /// Returns the last block in the pending queue, or the last accumulated block,
    /// or the previous batch end. Used to determine the starting point for fetching
    /// new blocks.
    pub fn last_known_block(&self) -> Hash {
        self.pending_blocks
            .back()
            .copied()
            .or_else(|| self.accumulator.last_block())
            .unwrap_or(self.prev_batch_end)
    }
}

/// Initialize batch builder state from storage.
///
/// If batches exist in storage, resumes from the last batch.
/// Otherwise, starts fresh from genesis.
pub async fn init_batch_builder_state<P: BatchPolicy>(
    genesis_hash: Hash,
    batch_storage: &impl BatchStorage,
) -> Result<BatchBuilderState<P>> {
    match batch_storage.get_latest_batch().await? {
        Some((batch, _status)) => Ok(BatchBuilderState::from_last_batch(
            batch.idx(),
            batch.last_block(),
        )),
        None => Ok(BatchBuilderState::from_genesis(genesis_hash)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_builder::block_count::BlockCountPolicy;

    fn test_hash(n: u8) -> Hash {
        Hash::from([n; 32])
    }

    #[test]
    fn test_from_genesis() {
        let genesis = test_hash(0);
        let state: BatchBuilderState<BlockCountPolicy> = BatchBuilderState::from_genesis(genesis);

        assert_eq!(state.prev_batch_end(), genesis);
        assert_eq!(state.next_batch_idx(), 0);
        assert!(state.accumulator().is_empty());
    }

    #[test]
    fn test_from_last_batch() {
        let last_block = test_hash(10);
        let state: BatchBuilderState<BlockCountPolicy> =
            BatchBuilderState::from_last_batch(5, last_block);

        assert_eq!(state.prev_batch_end(), last_block);
        assert_eq!(state.next_batch_idx(), 6);
        assert!(state.accumulator().is_empty());
    }

    #[test]
    fn test_advance_batch() {
        let genesis = test_hash(0);
        let mut state: BatchBuilderState<BlockCountPolicy> =
            BatchBuilderState::from_genesis(genesis);

        let new_end = test_hash(5);
        state.advance_batch(new_end);

        assert_eq!(state.prev_batch_end(), new_end);
        assert_eq!(state.next_batch_idx(), 1);
        assert!(state.accumulator().is_empty());
    }
}
