//! Data availability provider trait for batch lifecycle management.

use async_trait::async_trait;

use crate::{BatchId, L1DaBlockRef};

/// Status of a batch DA operation.
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

// ═══════════════════════════════════════════════════════════════════════════════
// Chunked Blob Publisher Interface
// ═══════════════════════════════════════════════════════════════════════════════

/// Status of a chunked blob publication.
#[derive(Debug, Clone)]
pub enum ChunkedBlobStatus {
    /// Publication is pending (submitted but not confirmed).
    Pending {
        /// Number of chunks confirmed so far.
        confirmed_chunks: u16,
        /// Total number of chunks.
        total_chunks: u16,
    },
    /// All chunks confirmed with sufficient depth.
    Confirmed {
        /// Wtxids of the reveal transactions (ordered by chunk index).
        chunk_wtxids: Vec<[u8; 32]>,
        /// L1 block heights where chunks were confirmed.
        confirmed_heights: Vec<u64>,
    },
    /// Publication has not been requested.
    NotFound,
    /// Permanent failure.
    Failed { reason: String },
}

/// Low-level interface for publishing chunked blobs to L1.
///
/// This trait abstracts the chunked envelope publication system, allowing the
/// EE DA publisher to be decoupled from the actual Bitcoin transaction handling.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait ChunkedBlobPublisher: Send + Sync {
    /// Submits a blob payload for chunked publication.
    ///
    /// Returns a unique blob identifier (typically SHA256 of the payload).
    async fn submit_blob(&self, tag: [u8; 4], payload: &[u8]) -> eyre::Result<[u8; 32]>;

    /// Checks the publication status of a blob.
    async fn check_blob_status(&self, blob_hash: &[u8; 32]) -> eyre::Result<ChunkedBlobStatus>;
}
