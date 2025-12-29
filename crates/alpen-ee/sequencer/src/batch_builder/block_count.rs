//! Block-count based batching policy implementation.

use super::{Accumulator, BatchPolicy, BatchSealingPolicy};

/// Block-count based batching policy.
#[derive(Debug)]
pub struct BlockCountPolicy;

/// Block data for block-count policy.
///
/// No additional data is needed since the count is tracked by the
/// accumulator's block list.
#[derive(Debug, Clone, Default)]
pub struct BlockCountData;

/// Accumulated value for block-count policy.
///
/// This is a unit type since the block count is tracked by
/// `Accumulator::block_count()` directly.
#[derive(Debug, Default)]
pub struct BlockCountValue;

impl BatchPolicy for BlockCountPolicy {
    type BlockData = BlockCountData;
    type AccumulatedValue = BlockCountValue;

    fn accumulate(_value: &mut Self::AccumulatedValue, _data: &Self::BlockData) {
        // No-op: block count is tracked by Accumulator::blocks.len()
    }
}

/// Fixed block count sealing policy.
///
/// Seals a batch when the number of blocks reaches the configured maximum.
#[derive(Debug)]
pub struct FixedBlockCountSealing {
    max_blocks: u64,
}

impl FixedBlockCountSealing {
    /// Create a new fixed block count sealing policy.
    ///
    /// # Arguments
    ///
    /// * `max_blocks` - Maximum number of blocks per batch
    pub fn new(max_blocks: u64) -> Self {
        Self { max_blocks }
    }

    /// Get the maximum blocks per batch.
    pub fn max_blocks(&self) -> u64 {
        self.max_blocks
    }
}

impl BatchSealingPolicy<BlockCountPolicy> for FixedBlockCountSealing {
    fn would_exceed(
        &self,
        accumulator: &Accumulator<BlockCountPolicy>,
        _block_data: &BlockCountData,
    ) -> bool {
        // Seal if we've already reached max_blocks
        accumulator.block_count() >= self.max_blocks
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::Hash;

    use super::*;

    fn test_hash(n: u8) -> Hash {
        Hash::from([n; 32])
    }

    #[test]
    fn test_would_not_exceed_when_empty() {
        let sealing = FixedBlockCountSealing::new(3);
        let accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        assert!(!sealing.would_exceed(&accumulator, &BlockCountData));
    }

    #[test]
    fn test_would_not_exceed_below_max() {
        let sealing = FixedBlockCountSealing::new(3);
        let mut accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        accumulator.add_block(test_hash(1), &BlockCountData);
        assert!(!sealing.would_exceed(&accumulator, &BlockCountData));

        accumulator.add_block(test_hash(2), &BlockCountData);
        assert!(!sealing.would_exceed(&accumulator, &BlockCountData));
    }

    #[test]
    fn test_would_exceed_at_max() {
        let sealing = FixedBlockCountSealing::new(3);
        let mut accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        accumulator.add_block(test_hash(1), &BlockCountData);
        accumulator.add_block(test_hash(2), &BlockCountData);
        accumulator.add_block(test_hash(3), &BlockCountData);

        // Now at 3 blocks, adding another would exceed
        assert!(sealing.would_exceed(&accumulator, &BlockCountData));
    }

    #[test]
    fn test_max_blocks_getter() {
        let sealing = FixedBlockCountSealing::new(100);
        assert_eq!(sealing.max_blocks(), 100);
    }
}
