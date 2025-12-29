//! Accumulator for pending batch blocks and policy-specific values.

use strata_acct_types::Hash;

use super::BatchPolicy;

/// Accumulates blocks and policy-specific value for the pending batch.
#[derive(Debug)]
pub struct Accumulator<P: BatchPolicy> {
    /// Blocks accumulated so far (in order)
    blocks: Vec<Hash>,
    /// Policy-specific accumulated value
    value: P::AccumulatedValue,
}

impl<P: BatchPolicy> Default for Accumulator<P> {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            value: P::AccumulatedValue::default(),
        }
    }
}

impl<P: BatchPolicy> Accumulator<P> {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a block to the accumulator.
    ///
    /// This appends the block hash to the list and calls the policy's
    /// accumulate function to update the accumulated value.
    pub fn add_block(&mut self, hash: Hash, data: &P::BlockData) {
        self.blocks.push(hash);
        P::accumulate(&mut self.value, data);
    }

    /// Number of blocks accumulated.
    pub fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    /// All accumulated block hashes in order.
    pub fn blocks(&self) -> &[Hash] {
        &self.blocks
    }

    /// Last block hash, if any.
    pub fn last_block(&self) -> Option<Hash> {
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

    /// Reset accumulator for a new batch.
    pub fn reset(&mut self) {
        self.blocks.clear();
        self.value = P::AccumulatedValue::default();
    }

    /// Drain blocks for batch creation.
    ///
    /// Returns `(inner_blocks, last_block)` where `inner_blocks` excludes `last_block`.
    ///
    /// # Panics
    ///
    /// Panics if the accumulator is empty.
    pub fn drain_for_batch(&mut self) -> (Vec<Hash>, Hash) {
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

    // Simple test policy for unit tests
    struct TestPolicy;

    #[derive(Debug, Clone)]
    struct TestBlockData {
        value: u64,
    }

    #[derive(Debug, Default)]
    struct TestAccumulatedValue {
        total: u64,
    }

    impl BatchPolicy for TestPolicy {
        type BlockData = TestBlockData;
        type AccumulatedValue = TestAccumulatedValue;

        fn accumulate(value: &mut Self::AccumulatedValue, data: &Self::BlockData) {
            value.total += data.value;
        }
    }

    fn test_hash(n: u8) -> Hash {
        Hash::from([n; 32])
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
        let hash = test_hash(1);
        let data = TestBlockData { value: 10 };

        acc.add_block(hash, &data);

        assert!(!acc.is_empty());
        assert_eq!(acc.block_count(), 1);
        assert_eq!(acc.last_block(), Some(hash));
        assert_eq!(acc.value().total, 10);
    }

    #[test]
    fn test_add_multiple_blocks() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();

        acc.add_block(test_hash(1), &TestBlockData { value: 10 });
        acc.add_block(test_hash(2), &TestBlockData { value: 20 });
        acc.add_block(test_hash(3), &TestBlockData { value: 30 });

        assert_eq!(acc.block_count(), 3);
        assert_eq!(acc.last_block(), Some(test_hash(3)));
        assert_eq!(acc.value().total, 60);
        assert_eq!(acc.blocks(), &[test_hash(1), test_hash(2), test_hash(3)]);
    }

    #[test]
    fn test_reset() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_hash(1), &TestBlockData { value: 10 });
        acc.add_block(test_hash(2), &TestBlockData { value: 20 });

        acc.reset();

        assert!(acc.is_empty());
        assert_eq!(acc.block_count(), 0);
        assert_eq!(acc.value().total, 0);
    }

    #[test]
    fn test_drain_for_batch() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_hash(1), &TestBlockData { value: 10 });
        acc.add_block(test_hash(2), &TestBlockData { value: 20 });
        acc.add_block(test_hash(3), &TestBlockData { value: 30 });

        let (inner, last) = acc.drain_for_batch();

        assert_eq!(inner, vec![test_hash(1), test_hash(2)]);
        assert_eq!(last, test_hash(3));
        assert!(acc.is_empty());
        assert_eq!(acc.value().total, 0);
    }

    #[test]
    fn test_drain_single_block() {
        let mut acc: Accumulator<TestPolicy> = Accumulator::new();
        acc.add_block(test_hash(1), &TestBlockData { value: 10 });

        let (inner, last) = acc.drain_for_batch();

        assert!(inner.is_empty());
        assert_eq!(last, test_hash(1));
    }
}
