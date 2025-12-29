//! Core trait that defines the types for a batching strategy.

use std::fmt::Debug;

use async_trait::async_trait;
use strata_acct_types::Hash;

use super::Accumulator;

/// Core trait that defines the types for a batching strategy.
///
/// The `BatchPolicy` trait allows different batching strategies to be implemented
/// by specifying the types for block data and accumulated values, along with
/// the accumulation logic.
pub trait BatchPolicy: Send + Sync + 'static {
    /// Data collected per block, used for sealing decisions.
    /// This is provided by [`BlockDataProvider`](super::BlockDataProvider).
    type BlockData: Send + Sync + Clone;

    /// Accumulated value across blocks (e.g., count, DA size).
    /// Must implement `Default` for initialization and reset.
    type AccumulatedValue: Default + Send + Sync + Debug;

    /// Accumulate block data into the value.
    ///
    /// Called when a block is added to the accumulator.
    fn accumulate(value: &mut Self::AccumulatedValue, data: &Self::BlockData);
}

/// Policy for deciding when to seal a batch.
///
/// Implementations define the threshold logic for determining when a batch
/// should be sealed (e.g., by block count, DA size, or a combination).
pub trait BatchSealingPolicy<P: BatchPolicy>: Send + Sync {
    /// Check if adding a block would exceed the batch threshold.
    ///
    /// If this returns `true`, the current batch should be sealed before
    /// adding this block. The block will then become the first block of
    /// the next batch.
    ///
    /// This is called with the current accumulator state and the data for
    /// the block about to be added.
    fn would_exceed(&self, accumulator: &Accumulator<P>, block_data: &P::BlockData) -> bool;
}

/// Trait to fetch processed block data for batch sealing.
#[async_trait]
pub trait BlockDataProvider<P: BatchPolicy>: Send + Sync {
    /// Get processed data for a block.
    ///
    /// Returns `None` if data is not yet available (block still processing).
    /// The caller should retry after a delay if `None` is returned.
    async fn get_block_data(&self, hash: Hash) -> eyre::Result<Option<P::BlockData>>;
}
