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
    /// Post DA data for a batch.
    ///
    /// Returns the transaction IDs that were broadcast. These are used to track
    /// confirmation status.
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<()>;

    /// Check DA status for pending transactions.
    ///
    /// Returns `None` if transactions are still pending confirmation.
    /// Returns `Some(refs)` when all transactions are confirmed in L1 blocks.
    async fn check_da_status(&self, batch_id: BatchId) -> eyre::Result<DaStatus>;
}
