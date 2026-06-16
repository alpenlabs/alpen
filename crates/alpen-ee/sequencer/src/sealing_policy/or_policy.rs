//! Combinators for composing batch policies and sealing strategies.
//!
//! [`ComposedPolicy`] pairs two [`AccumulationPolicy`] types into one whose block data
//! and accumulated value are tuples of the inner types. Sealing combinators
//! like [`OrSealing`] define how the two halves are checked.
//! [`ComposedDataProvider`] fetches block data from both inner providers.

use std::marker::PhantomData;

use async_trait::async_trait;
use strata_acct_types::Hash;

use super::policy::{AccumulationPolicy, BlockDataProvider, SealingPolicy};

/// Composed batch policy that pairs two inner policies.
///
/// `BlockData` is `(A::BlockData, B::BlockData)` and `AccumulatedValue` is
/// `(A::AccumulatedValue, B::AccumulatedValue)`. Each half is accumulated
/// independently.
#[derive(Debug)]
pub struct ComposedPolicy<A: AccumulationPolicy, B: AccumulationPolicy>(PhantomData<(A, B)>);

impl<A: AccumulationPolicy, B: AccumulationPolicy> AccumulationPolicy for ComposedPolicy<A, B> {
    type BlockData = (A::BlockData, B::BlockData);
    type AccumulatedValue = (A::AccumulatedValue, B::AccumulatedValue);

    fn accumulate(value: &mut Self::AccumulatedValue, data: &Self::BlockData) {
        A::accumulate(&mut value.0, &data.0);
        B::accumulate(&mut value.1, &data.1);
    }
}

/// Seals a batch when **either** of two sealing policies triggers.
///
/// Both `SA` and `SB` operate on their respective projected half of the
/// composed accumulated value.
#[derive(Debug)]
pub struct OrSealing<A: AccumulationPolicy, B: AccumulationPolicy, SA, SB> {
    a: SA,
    b: SB,
    _marker: PhantomData<(A, B)>,
}

impl<A, B, SA, SB> OrSealing<A, B, SA, SB>
where
    A: AccumulationPolicy,
    B: AccumulationPolicy,
    SA: SealingPolicy<A>,
    SB: SealingPolicy<B>,
{
    /// Create a new OR-combined sealing policy.
    pub fn new(a: SA, b: SB) -> Self {
        Self {
            a,
            b,
            _marker: PhantomData,
        }
    }
}

impl<A, B, SA, SB> SealingPolicy<ComposedPolicy<A, B>> for OrSealing<A, B, SA, SB>
where
    A: AccumulationPolicy,
    B: AccumulationPolicy,
    SA: SealingPolicy<A>,
    SB: SealingPolicy<B>,
{
    fn would_exceed(
        &self,
        value: &(A::AccumulatedValue, B::AccumulatedValue),
        block_data: &(A::BlockData, B::BlockData),
    ) -> bool {
        self.a.would_exceed(&value.0, &block_data.0) || self.b.would_exceed(&value.1, &block_data.1)
    }
}

/// Composed data provider that fetches block data from two inner providers.
#[derive(Debug)]
pub struct ComposedDataProvider<A: AccumulationPolicy, B: AccumulationPolicy, DA, DB> {
    a: DA,
    b: DB,
    _marker: PhantomData<(A, B)>,
}

impl<A, B, DA, DB> ComposedDataProvider<A, B, DA, DB>
where
    A: AccumulationPolicy,
    B: AccumulationPolicy,
    DA: BlockDataProvider<A>,
    DB: BlockDataProvider<B>,
{
    /// Create a new composed data provider.
    pub fn new(a: DA, b: DB) -> Self {
        Self {
            a,
            b,
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<A, B, DA, DB> BlockDataProvider<ComposedPolicy<A, B>> for ComposedDataProvider<A, B, DA, DB>
where
    A: AccumulationPolicy,
    B: AccumulationPolicy,
    DA: BlockDataProvider<A>,
    DB: BlockDataProvider<B>,
{
    async fn get_block_data(
        &self,
        hash: Hash,
    ) -> eyre::Result<Option<(A::BlockData, B::BlockData)>> {
        let (a, b) = tokio::try_join!(self.a.get_block_data(hash), self.b.get_block_data(hash))?;
        Ok(a.zip(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sealing_policy::{
            block_count_policy::{BlockCountData, BlockCountPolicy, FixedBlockCountSealing},
            gas_limit_policy::{GasBlockData, GasLimitPolicy, MaxGasSealing},
            policy::Accumulator,
        },
        test_utils::*,
    };

    type Combined = ComposedPolicy<BlockCountPolicy, GasLimitPolicy>;

    fn block_data(gas: u64) -> (BlockCountData, GasBlockData) {
        (BlockCountData, GasBlockData { gas_used: gas })
    }

    #[test]
    fn test_seals_on_block_count() {
        // max 2 blocks, high gas limit
        let sealing = OrSealing::new(FixedBlockCountSealing::new(2), MaxGasSealing::new(10_000));
        let mut acc: Accumulator<Combined> = Accumulator::new();

        // 1 accumulated + 1 incoming = 2 <= 2, no seal
        acc.add_block(test_blocknumhash(1), &block_data(10));
        assert!(!acc.would_exceed(&sealing, &block_data(10)));

        // 2 accumulated + 1 incoming = 3 > 2, seal on count
        acc.add_block(test_blocknumhash(2), &block_data(10));
        assert!(acc.would_exceed(&sealing, &block_data(10)));
    }

    #[test]
    fn test_seals_on_gas() {
        // high block count, max 50 gas
        let sealing = OrSealing::new(FixedBlockCountSealing::new(100), MaxGasSealing::new(50));
        let mut acc: Accumulator<Combined> = Accumulator::new();

        // 0 + 40 = 40 <= 50, no seal
        assert!(!acc.would_exceed(&sealing, &block_data(40)));

        // 40 accumulated + 20 incoming = 60 > 50, seal on gas
        acc.add_block(test_blocknumhash(1), &block_data(40));
        assert!(acc.would_exceed(&sealing, &block_data(20)));
    }

    #[test]
    fn test_neither_seals() {
        let sealing = OrSealing::new(FixedBlockCountSealing::new(100), MaxGasSealing::new(1000));
        let mut acc: Accumulator<Combined> = Accumulator::new();

        // 0 + 1 = 1 block (<< 100), 0 + 10 = 10 gas (<< 1000)
        acc.add_block(test_blocknumhash(1), &block_data(10));
        assert!(!acc.would_exceed(&sealing, &block_data(10)));
    }

    /// When gas limit is `u64::MAX` the gas policy never fires, so only
    /// block count matters. This mirrors the production path when
    /// `--chunk-sealing-gas-limit` is omitted.
    #[test]
    fn test_gas_disabled_via_max() {
        let sealing = OrSealing::new(FixedBlockCountSealing::new(3), MaxGasSealing::new(u64::MAX));
        let mut acc: Accumulator<Combined> = Accumulator::new();

        // Accumulate huge gas — still shouldn't seal until block count exceeds 3
        acc.add_block(test_blocknumhash(1), &block_data(u64::MAX / 4));
        acc.add_block(test_blocknumhash(2), &block_data(u64::MAX / 4));
        // 2 + 1 = 3 <= 3, no seal
        assert!(!acc.would_exceed(&sealing, &block_data(0)));

        // 3 + 1 = 4 > 3, seal on count
        acc.add_block(test_blocknumhash(3), &block_data(0));
        assert!(acc.would_exceed(&sealing, &block_data(0)));
    }

    /// After draining the accumulator (sealing a batch), both halves of the
    /// composed value reset so the next batch starts fresh.
    #[test]
    fn test_drain_resets_both_values() {
        let sealing = OrSealing::new(FixedBlockCountSealing::new(2), MaxGasSealing::new(100));
        let mut acc: Accumulator<Combined> = Accumulator::new();

        acc.add_block(test_blocknumhash(1), &block_data(80));
        acc.add_block(test_blocknumhash(2), &block_data(80));

        // count: 2 + 1 = 3 > 2, gas: 160 + 10 = 170 > 100 — both would seal
        assert!(acc.would_exceed(&sealing, &block_data(10)));

        // Drain (seal the batch)
        let (inner, last) = acc.drain();
        assert_eq!(inner.len(), 1);
        assert_eq!(last, test_blocknumhash(2));

        // After drain, both counters are zero — neither policy seals
        assert!(!acc.would_exceed(&sealing, &block_data(0)));
        assert_eq!(acc.value().0.count, 0);
        assert_eq!(acc.value().1.total_gas, 0);
    }
}
