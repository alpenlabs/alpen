//! Data availability provider trait for batch lifecycle management.

use async_trait::async_trait;
use bitcoin::{Txid, Wtxid};

use crate::{BatchId, L1DaBlockRef};

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
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<Vec<(Txid, Wtxid)>>;

    /// Check DA status for pending transactions.
    ///
    /// Returns `None` if transactions are still pending confirmation.
    /// Returns `Some(refs)` when all transactions are confirmed in L1 blocks.
    async fn check_da_status(
        &self,
        txns: &[(Txid, Wtxid)],
    ) -> eyre::Result<Option<Vec<L1DaBlockRef>>>;
}
