//! [`DaBlobSource`] — the seam between DA blob assembly and publication.

use alpen_ee_common::BatchId;
use alpen_ee_da_types::DaBlob;
use async_trait::async_trait;

/// Source of [`DaBlob`]s for a batch, and owner of the cross-batch DA dedup
/// filter.
///
/// Encapsulates readiness checking (are the underlying state diffs available?),
/// blob assembly, and recording what has been published — separating data
/// preparation ([`StateDiffBlobProvider`](crate::StateDiffBlobProvider)) from
/// publication: encoding, chunking, posting to Bitcoin, tracking
/// ([`ChunkedEnvelopeDaProvider`](crate::ChunkedEnvelopeDaProvider)).
#[async_trait]
pub trait DaBlobSource: Send + Sync {
    /// Returns the [`DaBlob`] for the given batch.
    ///
    /// The blob contains batch metadata and the aggregated state diff.
    /// Even batches with no state changes return a blob (with empty state diff)
    /// to ensure L1 chain continuity.
    async fn get_blob(&self, batch_id: BatchId) -> eyre::Result<DaBlob>;

    /// Returns `true` if state diffs are ready for all blocks in the given batch.
    ///
    /// Used by [`ChunkedEnvelopeDaProvider`](crate::ChunkedEnvelopeDaProvider)
    /// to ensure state diffs have been written by the Reth exex before posting
    /// DA. This prevents race conditions where DA posting is attempted before
    /// state diffs are ready.
    async fn are_state_diffs_ready(&self, batch_id: BatchId) -> bool;

    /// Records the batch's published data in the cross-batch dedup filter, so
    /// future batches omit already-published items (currently deployed
    /// bytecodes).
    ///
    /// Invoked by [`ChunkedEnvelopeDaProvider`](crate::ChunkedEnvelopeDaProvider)
    /// once the batch's DA reaches reorg-safe finality. The filter has no L1
    /// reorg rollback path, so this must only run after the publishing
    /// transactions are final.
    async fn mark_batch_published(&self, batch_id: BatchId) -> eyre::Result<()>;
}
