//! Policy traits and accumulator for block accumulation and sealing.
//!
//! These types define a generic framework for accumulating blocks and
//! deciding when to seal a group (batch, chunk, or any other contiguous
//! block range). Both the batch builder and chunk builder use these with
//! different policy implementations.

use std::fmt::Debug;

use alpen_ee_common::BlockNumHash;
use async_trait::async_trait;
use strata_acct_types::Hash;

/// Core trait that defines the types for an accumulation strategy.
///
/// The policy specifies what data is collected per block and how it's
/// accumulated across a contiguous range of blocks (e.g., a batch for
/// DA submission, a chunk for proving).
pub trait AccumulationPolicy: Send + Sync + 'static {
    /// Data collected per block, used for sealing decisions.
    /// This is provided by [`BlockDataProvider`].
    type BlockData: Send + Sync + Clone;

    /// Accumulated value across blocks (e.g., count, DA size).
    /// Must implement [`Default`] for initialization and reset.
    type AccumulatedValue: Default + Send + Sync + Debug;

    /// Accumulate block data into the value.
    ///
    /// Called when a block is added to the accumulator.
    fn accumulate(value: &mut Self::AccumulatedValue, data: &Self::BlockData);
}

/// Policy for deciding when to seal a group of accumulated blocks.
///
/// Implementations define the threshold logic (e.g., by block count,
/// DA size, prover cost, or a combination).
pub trait SealingPolicy<P: AccumulationPolicy>: Send + Sync {
    /// Check if adding a block would exceed the threshold.
    ///
    /// If this returns `true`, the current group should be sealed before
    /// adding this block. The block will then become the first block of
    /// the next group.
    fn would_exceed(&self, accumulator: &Accumulator<P>, block_data: &P::BlockData) -> bool;
}

/// Trait to fetch processed block data for sealing decisions.
#[async_trait]
pub trait BlockDataProvider<P: AccumulationPolicy>: Send + Sync {
    /// Get processed data for a block.
    ///
    /// Returns `None` if data is not yet available (block still processing).
    /// The caller should retry after a delay if `None` is returned.
    async fn get_block_data(&self, hash: Hash) -> eyre::Result<Option<P::BlockData>>;
}

// ---------------------------------------------------------------------------
// Accumulator
// ---------------------------------------------------------------------------

/// Accumulates blocks and a policy-specific value for a pending group.
///
/// Used by both the batch builder and chunk builder to track blocks
/// before sealing.
#[derive(Debug)]
pub struct Accumulator<P: AccumulationPolicy> {
    /// Blocks accumulated so far (in order).
    blocks: Vec<BlockNumHash>,
    /// Policy-specific accumulated value.
    value: P::AccumulatedValue,
}

impl<P: AccumulationPolicy> Default for Accumulator<P> {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            value: P::AccumulatedValue::default(),
        }
    }
}

impl<P: AccumulationPolicy> Accumulator<P> {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a block to the accumulator.
    ///
    /// Appends the block to the list and calls the policy's accumulate
    /// function to update the accumulated value.
    pub fn add_block(&mut self, block: BlockNumHash, data: &P::BlockData) {
        self.blocks.push(block);
        P::accumulate(&mut self.value, data);
    }

    /// Number of blocks accumulated.
    pub fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    /// All accumulated block hashes in order.
    pub fn blocks(&self) -> &[BlockNumHash] {
        &self.blocks
    }

    /// Last block hash, if any.
    pub fn last_block(&self) -> Option<BlockNumHash> {
        self.blocks.last().copied()
    }

    /// Whether accumulator is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Access the accumulated value.
    pub fn value(&self) -> &P::AccumulatedValue {
        &self.value
    }

    /// Reset accumulator for a new group.
    pub fn reset(&mut self) {
        self.blocks.clear();
        self.value = P::AccumulatedValue::default();
    }

    /// Drain blocks for group creation.
    ///
    /// Returns `(inner_blocks, last_block)` where `inner_blocks` excludes
    /// `last_block`.
    ///
    /// # Panics
    ///
    /// Panics if the accumulator is empty.
    #[allow(clippy::absolute_paths, clippy::allow_attributes, reason = "std")]
    pub fn drain(&mut self) -> (Vec<BlockNumHash>, BlockNumHash) {
        debug_assert!(!self.blocks.is_empty(), "Cannot drain empty accumulator");
        let last = self.blocks.pop().expect("accumulator is not empty");
        let inner = std::mem::take(&mut self.blocks);
        self.value = P::AccumulatedValue::default();
        (inner, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    struct TestPolicy;

    #[derive(Debug, Clone)]
    struct TestBlockData {
        value: u64,
    }

    #[derive(Debug, Default)]
    struct TestAccumulatedValue {
        total: u64,
    }

    impl AccumulationPolicy for TestPolicy {
        type BlockData = TestBlockData;
        type AccumulatedValue = TestAccumulatedValue;

        fn accumulate(value: &mut Self::AccumulatedValue, data: &Self::BlockData) {
            value.total += data.value;
        }
    }

    #[test]
    fn test_new_accumulator_is_empty() {
        let acc: Accumulator<TestPolicy> = Accumulator::new();
        assert!(acc.is_empty());
        assert_eq!(acc.block_count(), 0);
        assert!(acc.last_block().is_none());
        assert_eq!(acc.value().total, 0);
    }

    #[test]
    fn test_add_block() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        let block = test_blocknumhash(1);
        let data = TestBlockData { value: 10 };

        acc.add_block(block, &data);

        assert!(!acc.is_empty());
        assert_eq!(acc.block_count(), 1);
        assert_eq!(acc.last_block(), Some(block));
        assert_eq!(acc.value().total, 10);
    }

    #[test]
    fn test_add_multiple_blocks() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();

        acc.add_block(test_blocknumhash(1), &TestBlockData { value: 10 });
        acc.add_block(test_blocknumhash(2), &TestBlockData { value: 20 });
        acc.add_block(test_blocknumhash(3), &TestBlockData { value: 30 });

        assert_eq!(acc.block_count(), 3);
        assert_eq!(acc.last_block(), Some(test_blocknumhash(3)));
        assert_eq!(acc.value().total, 60);
        assert_eq!(
            acc.blocks(),
            &[
                test_blocknumhash(1),
                test_blocknumhash(2),
                test_blocknumhash(3)
            ]
        );
    }

    #[test]
    fn test_reset() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_blocknumhash(1), &TestBlockData { value: 10 });
        acc.add_block(test_blocknumhash(2), &TestBlockData { value: 20 });

        acc.reset();

        assert!(acc.is_empty());
        assert_eq!(acc.block_count(), 0);
        assert_eq!(acc.value().total, 0);
    }

    #[test]
    fn test_drain() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_blocknumhash(1), &TestBlockData { value: 10 });
        acc.add_block(test_blocknumhash(2), &TestBlockData { value: 20 });
        acc.add_block(test_blocknumhash(3), &TestBlockData { value: 30 });

        let (inner, last) = acc.drain();

        assert_eq!(inner, vec![test_blocknumhash(1), test_blocknumhash(2)]);
        assert_eq!(last, test_blocknumhash(3));
        assert!(acc.is_empty());
        assert_eq!(acc.value().total, 0);
    }

    #[test]
    fn test_drain_single_block() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_blocknumhash(1), &TestBlockData { value: 10 });

        let (inner, last) = acc.drain();

        assert!(inner.is_empty());
        assert_eq!(last, test_blocknumhash(1));
    }
}
