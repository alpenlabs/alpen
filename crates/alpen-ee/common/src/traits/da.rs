//! Data availability provider trait for batch lifecycle management.

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
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<()>;

    /// Checks DA status for a batch.
    ///
    /// Returns a [`DaStatus`] indicating whether DA is pending, ready with L1
    /// block references, not yet requested, or has permanently failed.
    async fn check_da_status(&self, batch_id: BatchId) -> eyre::Result<DaStatus>;
}
