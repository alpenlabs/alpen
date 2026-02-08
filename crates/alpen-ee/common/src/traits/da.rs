//! Data availability provider trait for batch lifecycle management.

use async_trait::async_trait;

use crate::{BatchId, DaBlob, L1DaBlockRef};

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
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<()>;

    /// Checks DA status for a batch.
    ///
    /// Returns a [`DaStatus`] indicating whether DA is pending, ready with L1
    /// block references, not yet requested, or has permanently failed.
    async fn check_da_status(&self, batch_id: BatchId) -> eyre::Result<DaStatus>;
}

/// Provides the DA blob for a batch.
///
/// Separates blob preparation (fetching state diffs, aggregating)
/// from blob publication (encoding, chunking, posting to Bitcoin, tracking).
#[cfg_attr(feature = "test-utils", mockall::automock)]
pub trait DaBlobProvider: Send + Sync {
    /// Returns the [`DaBlob`] for the given batch.
    ///
    /// The blob contains batch metadata and the aggregated state diff.
    /// Even batches with no state changes return a blob (with empty state diff)
    /// to ensure L1 chain continuity.
    fn get_blob(&self, batch_id: BatchId) -> eyre::Result<DaBlob>;

    /// Returns `true` if state diffs are ready for all blocks in the given batch.
    ///
    /// Used by the batch lifecycle to ensure state diffs have been written
    /// by the Reth exex before attempting to post DA. This prevents race
    /// conditions where DA posting is attempted before state diffs are ready.
    fn are_state_diffs_ready(&self, batch_id: BatchId) -> eyre::Result<bool>;
}
