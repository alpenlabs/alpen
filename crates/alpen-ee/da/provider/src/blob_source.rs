//! [`DaBlobSource`] — the seam between DA blob assembly and publication.

use alpen_ee_common::BatchId;
use alpen_ee_da_types::DaBlob;
use async_trait::async_trait;

/// Source of [`DaBlob`]s for a batch.
///
/// Encapsulates both readiness checking (are the underlying state diffs
/// available?) and blob assembly, separating data preparation
/// ([`StateDiffBlobProvider`](crate::StateDiffBlobProvider)) from publication —
/// encoding, chunking, posting to Bitcoin, tracking
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
}
