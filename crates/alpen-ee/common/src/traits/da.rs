//! Data availability provider trait for batch lifecycle management.

use alpen_ee_da_types::EvmHeaderSummary;
use async_trait::async_trait;

use crate::{BatchId, L1DaBlockRef};

#[derive(Debug)]
pub enum DaStatus {
    /// DA requested and operation is pending.
    /// Temporary failures are retried internally while status remains `Pending`.
    Pending,
    /// DA is included in blocks with sufficient depth.
    Ready(Vec<L1DaBlockRef>),
    /// DA has not been requested for this [`BatchId`].
    NotRequested,
    /// Permanent failure that cannot be handled automatically.
    /// Needs manual intervention to resolve.
    Failed { reason: String },
}

/// Interface for posting and checking batch data availability.
///
/// This trait abstracts the DA layer interaction, allowing the batch lifecycle
/// manager to be decoupled from the actual DA implementation.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BatchDaProvider: Send + Sync {
    /// Posts DA data for a batch.
    ///
    /// Initiates the data availability posting process for the given batch.
    /// The implementation handles broadcasting and internal tracking.
    /// Returns the chunked envelope index assigned to this DA submission.
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<u64>;

    /// Checks DA status for a batch.
    ///
    /// The `envelope_idx` identifies the chunked envelope entry assigned when
    /// DA was first posted. It is persisted in
    /// [`BatchStatus::DaPending`](crate::BatchStatus::DaPending) so the caller can supply it
    /// even after a restart.
    ///
    /// Returns a [`DaStatus`] indicating whether DA is pending, ready with L1
    /// block references, not yet requested, or has permanently failed.
    async fn check_da_status(&self, batch_id: BatchId, envelope_idx: u64)
        -> eyre::Result<DaStatus>;

    /// Notifies the provider that the batch's DA reached completion, allowing it to perform
    /// DA-internal bookkeeping such as cross-batch deduplication.
    ///
    /// Invoked once by the batch lifecycle on the `DaPending -> DaComplete`
    /// transition, before the new status is persisted. On error the batch stays
    /// in `DaPending` and the lifecycle retries, so implementations must be
    /// idempotent.
    async fn confirm_da_complete(&self, batch_id: BatchId) -> eyre::Result<()>;
}

/// Provides EVM block header summaries by block number.
///
/// Used during DA blob construction to attach chain-reconstruction metadata
/// to each batch. The binary crate supplies the concrete implementation
/// backed by its block header store.
pub trait HeaderSummaryProvider: Send + Sync {
    /// Returns the [`EvmHeaderSummary`] for the given block number.
    fn header_summary(&self, block_num: u64) -> eyre::Result<EvmHeaderSummary>;
}
