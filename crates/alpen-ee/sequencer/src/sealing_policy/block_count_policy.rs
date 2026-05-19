//! Block-count based batching policy implementation.

use async_trait::async_trait;
use strata_acct_types::Hash;

use super::policy::{AccumulationPolicy, BlockDataProvider, SealingPolicy};

/// Block-count based batching policy.
#[derive(Debug)]
pub struct BlockCountPolicy;

/// Block data for block-count policy.
///
/// No additional data is needed; each block increments the count by one.
#[derive(Debug, Clone, Default)]
pub struct BlockCountData;

/// Accumulated block count.
#[derive(Debug, Default)]
pub struct BlockCountValue {
    /// Number of blocks accumulated so far.
    pub count: u64,
}

impl AccumulationPolicy for BlockCountPolicy {
    type BlockData = BlockCountData;
    type AccumulatedValue = BlockCountValue;

    fn accumulate(value: &mut Self::AccumulatedValue, _data: &Self::BlockData) {
        value.count += 1;
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

impl SealingPolicy<BlockCountPolicy> for FixedBlockCountSealing {
    fn would_exceed(&self, value: &BlockCountValue, _block_data: &BlockCountData) -> bool {
        // Each block contributes exactly 1 to the count.
        value.count + 1 > self.max_blocks
    }
}

/// Data provider for [`BlockCountPolicy`].
///
/// Doesn't need any data, so its just a stub to satisfy the trait.
#[derive(Debug)]
pub struct BlockCountDataProvider;

#[async_trait]
impl BlockDataProvider<BlockCountPolicy> for BlockCountDataProvider {
    async fn get_block_data(&self, _hash: Hash) -> eyre::Result<Option<BlockCountData>> {
        // No additional data needed for BlockCountPolicy
        Ok(Some(BlockCountData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sealing_policy::Accumulator, test_utils::*};

    #[test]
    fn test_would_not_exceed_when_empty() {
        let sealing = FixedBlockCountSealing::new(3);
        let accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        assert!(!accumulator.would_exceed(&sealing, &BlockCountData));
    }

    #[test]
    fn test_would_not_exceed_below_max() {
        let sealing = FixedBlockCountSealing::new(3);
        let mut accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        // 0 + 1 = 1 <= 3
        accumulator.add_block(test_blocknumhash(1), &BlockCountData);
        assert!(!accumulator.would_exceed(&sealing, &BlockCountData));

        // 1 + 1 = 2 <= 3
        accumulator.add_block(test_blocknumhash(2), &BlockCountData);
        assert!(!accumulator.would_exceed(&sealing, &BlockCountData));
    }

    #[test]
    fn test_would_not_exceed_at_exact_max() {
        let sealing = FixedBlockCountSealing::new(3);
        let mut accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        accumulator.add_block(test_blocknumhash(1), &BlockCountData);
        accumulator.add_block(test_blocknumhash(2), &BlockCountData);

        // 2 + 1 = 3 <= 3, batch of exactly max_blocks is allowed
        assert!(!accumulator.would_exceed(&sealing, &BlockCountData));
    }

    #[test]
    fn test_would_exceed_past_max() {
        let sealing = FixedBlockCountSealing::new(3);
        let mut accumulator: Accumulator<BlockCountPolicy> = Accumulator::new();

        accumulator.add_block(test_blocknumhash(1), &BlockCountData);
        accumulator.add_block(test_blocknumhash(2), &BlockCountData);
        accumulator.add_block(test_blocknumhash(3), &BlockCountData);

        // 3 + 1 = 4 > 3, seal before adding the 4th
        assert!(accumulator.would_exceed(&sealing, &BlockCountData));
    }

    #[test]
    fn test_max_blocks_getter() {
        let sealing = FixedBlockCountSealing::new(100);
        assert_eq!(sealing.max_blocks(), 100);
    }
}
