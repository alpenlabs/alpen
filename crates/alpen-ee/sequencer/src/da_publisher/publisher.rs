//! EE DA Publisher implementation.

use std::sync::Arc;

use alpen_ee_common::{
    BatchDaProvider, BatchId, BatchStorage, ChunkedBlobPublisher, ChunkedBlobStatus, DaStatus,
    L1DaBlockRef, EE_DA_TAG,
};
use async_trait::async_trait;
use bitcoin::hashes::Hash as _;
use eyre::{Context, Result};
use strata_identifiers::L1BlockCommitment;
use strata_primitives::buf::Buf32;
use tracing::{debug, info, warn};

use super::tracker::DaTracker;

/// EE Data Availability Publisher.
///
/// Implements [`BatchDaProvider`] by using the chunked blob publication system.
/// Batch data is serialized and published as chunked envelopes to L1.
#[derive(Debug)]
pub struct EeDaPublisher<P, S> {
    /// Chunked blob publisher for L1 publication.
    publisher: Arc<P>,
    /// Batch storage for retrieving batch data.
    batch_storage: Arc<S>,
    /// Tracks BatchId → blob_hash mapping.
    tracker: DaTracker,
}

impl<P, S> EeDaPublisher<P, S>
where
    P: ChunkedBlobPublisher,
    S: BatchStorage,
{
    /// Creates a new EE DA publisher.
    pub fn new(publisher: Arc<P>, batch_storage: Arc<S>) -> Self {
        Self {
            publisher,
            batch_storage,
            tracker: DaTracker::new(),
        }
    }

    /// Creates a new EE DA publisher with a pre-existing tracker.
    ///
    /// This is useful for restoring state after a restart.
    pub fn with_tracker(publisher: Arc<P>, batch_storage: Arc<S>, tracker: DaTracker) -> Self {
        Self {
            publisher,
            batch_storage,
            tracker,
        }
    }

    /// Gets the tracker for external state management.
    pub fn tracker(&self) -> &DaTracker {
        &self.tracker
    }

    /// Encodes batch data for DA publication.
    ///
    /// The format is:
    /// - 32 bytes: prev_block hash
    /// - 32 bytes: last_block hash
    /// - 8 bytes: batch index (u64, little-endian)
    /// - remaining: inner block hashes (32 bytes each)
    async fn encode_batch_data(&self, batch_id: BatchId) -> Result<Vec<u8>> {
        let Some((batch, _status)) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await
            .context("failed to get batch from storage")?
        else {
            eyre::bail!("batch not found: {:?}", batch_id);
        };

        let inner_blocks = batch.inner_blocks();
        let capacity = 32 + 32 + 8 + (inner_blocks.len() * 32);
        let mut data = Vec::with_capacity(capacity);

        // prev_block hash
        data.extend_from_slice(batch.prev_block().as_ref());
        // last_block hash
        data.extend_from_slice(batch.last_block().as_ref());
        // batch index
        data.extend_from_slice(&batch.idx().to_le_bytes());
        // inner block hashes
        for block_hash in inner_blocks {
            data.extend_from_slice(block_hash.as_ref());
        }

        Ok(data)
    }
}

#[async_trait]
impl<P, S> BatchDaProvider for EeDaPublisher<P, S>
where
    P: ChunkedBlobPublisher,
    S: BatchStorage,
{
    async fn post_batch_da(&self, batch_id: BatchId) -> Result<()> {
        // Check if already submitted
        if let Some(existing_hash) = self.tracker.get_blob_hash(&batch_id) {
            debug!(
                ?batch_id,
                blob_hash = ?hex::encode(existing_hash),
                "batch DA already submitted"
            );
            return Ok(());
        }

        // Encode batch data
        let data = self.encode_batch_data(batch_id).await?;
        let data_len = data.len();

        info!(
            ?batch_id,
            data_len,
            "submitting batch DA via chunked blob"
        );

        // Submit to chunked blob publisher
        let blob_hash = self
            .publisher
            .submit_blob(EE_DA_TAG, &data)
            .await
            .context("failed to submit blob")?;

        // Record the mapping
        self.tracker.record_submission(batch_id, blob_hash);

        info!(
            ?batch_id,
            blob_hash = ?hex::encode(blob_hash),
            "batch DA submitted successfully"
        );

        Ok(())
    }

    async fn check_da_status(&self, batch_id: BatchId) -> Result<DaStatus> {
        // Look up the blob_hash
        let Some(blob_hash) = self.tracker.get_blob_hash(&batch_id) else {
            return Ok(DaStatus::NotRequested);
        };

        // Check publication status
        let status = self
            .publisher
            .check_blob_status(&blob_hash)
            .await
            .context("failed to check blob status")?;

        match status {
            ChunkedBlobStatus::Pending {
                confirmed_chunks,
                total_chunks,
            } => {
                debug!(
                    ?batch_id,
                    confirmed_chunks,
                    total_chunks,
                    "batch DA pending"
                );
                Ok(DaStatus::Pending)
            }
            ChunkedBlobStatus::Confirmed {
                chunk_wtxids,
                confirmed_heights,
            } => {
                info!(
                    ?batch_id,
                    chunks = chunk_wtxids.len(),
                    "batch DA confirmed"
                );

                // Build L1DaBlockRef for each confirmed height
                // For now, group all txns under a single block ref per unique height
                let da_refs = build_da_refs(&chunk_wtxids, &confirmed_heights);

                Ok(DaStatus::Ready(da_refs))
            }
            ChunkedBlobStatus::NotFound => {
                // This shouldn't happen if tracker is consistent
                warn!(
                    ?batch_id,
                    blob_hash = ?hex::encode(blob_hash),
                    "blob not found but was tracked"
                );
                Ok(DaStatus::NotRequested)
            }
            ChunkedBlobStatus::Failed { reason } => {
                warn!(?batch_id, %reason, "batch DA failed permanently");
                Ok(DaStatus::Failed { reason })
            }
        }
    }
}

/// Builds L1DaBlockRef entries from confirmed chunk data.
///
/// Groups chunks by their confirmed height and creates one ref per unique height.
fn build_da_refs(chunk_wtxids: &[[u8; 32]], confirmed_heights: &[u64]) -> Vec<L1DaBlockRef> {
    use std::collections::BTreeMap;

    // Group wtxids by height
    let mut by_height: BTreeMap<u64, Vec<(bitcoin::Txid, bitcoin::Wtxid)>> = BTreeMap::new();

    for (wtxid_bytes, &height) in chunk_wtxids.iter().zip(confirmed_heights.iter()) {
        // Convert bytes to bitcoin types using the Hash trait
        let wtxid = bitcoin::Wtxid::from_byte_array(*wtxid_bytes);
        // We don't have the txid, use zeros as placeholder since we primarily track wtxid
        let txid = bitcoin::Txid::from_byte_array([0u8; 32]);

        by_height.entry(height).or_default().push((txid, wtxid));
    }

    // Build refs (we don't have actual block hashes, use placeholder)
    // In a full implementation, we'd query the L1 for block info
    by_height
        .into_iter()
        .filter_map(|(height, txns)| {
            // Create a placeholder block commitment
            // The actual block hash would be retrieved from L1 state
            let blkid = Buf32::zero().into();
            let block = L1BlockCommitment::from_height_u64(height, blkid)?;
            Some(L1DaBlockRef::new(block, txns))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpen_ee_common::{Batch, BatchStatus, MockBatchStorage, MockChunkedBlobPublisher};
    use strata_acct_types::Hash;

    fn make_batch_id(prev: u8, last: u8) -> BatchId {
        BatchId::from_parts(Hash::from([prev; 32]), Hash::from([last; 32]))
    }

    fn make_batch(idx: u64, prev: u8, last: u8) -> Batch {
        Batch::new(
            idx,
            Hash::from([prev; 32]),
            Hash::from([last; 32]),
            idx * 100,
            vec![],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_post_and_check_da_pending() {
        let mut publisher_mock = MockChunkedBlobPublisher::new();
        let mut storage_mock = MockBatchStorage::new();

        let batch_id = make_batch_id(1, 2);
        let batch = make_batch(1, 1, 2);
        let blob_hash = [0x42; 32];

        // Setup expectations
        storage_mock
            .expect_get_batch_by_id()
            .returning(move |_| Ok(Some((batch.clone(), BatchStatus::Sealed))));

        publisher_mock
            .expect_submit_blob()
            .returning(move |_, _| Ok(blob_hash));

        publisher_mock.expect_check_blob_status().returning(move |_| {
            Ok(ChunkedBlobStatus::Pending {
                confirmed_chunks: 0,
                total_chunks: 1,
            })
        });

        let publisher = EeDaPublisher::new(Arc::new(publisher_mock), Arc::new(storage_mock));

        // Post DA
        publisher.post_batch_da(batch_id).await.unwrap();

        // Check status
        let status = publisher.check_da_status(batch_id).await.unwrap();
        assert!(matches!(status, DaStatus::Pending));
    }

    #[tokio::test]
    async fn test_check_da_not_requested() {
        let publisher_mock = MockChunkedBlobPublisher::new();
        let storage_mock = MockBatchStorage::new();

        let publisher = EeDaPublisher::new(Arc::new(publisher_mock), Arc::new(storage_mock));

        let batch_id = make_batch_id(1, 2);
        let status = publisher.check_da_status(batch_id).await.unwrap();
        assert!(matches!(status, DaStatus::NotRequested));
    }

    #[tokio::test]
    async fn test_post_da_idempotent() {
        let mut publisher_mock = MockChunkedBlobPublisher::new();
        let mut storage_mock = MockBatchStorage::new();

        let batch_id = make_batch_id(1, 2);
        let batch = make_batch(1, 1, 2);
        let blob_hash = [0x42; 32];

        // Setup expectations - submit_blob should only be called once
        storage_mock
            .expect_get_batch_by_id()
            .times(1)
            .returning(move |_| Ok(Some((batch.clone(), BatchStatus::Sealed))));

        publisher_mock
            .expect_submit_blob()
            .times(1)
            .returning(move |_, _| Ok(blob_hash));

        let publisher = EeDaPublisher::new(Arc::new(publisher_mock), Arc::new(storage_mock));

        // First call submits
        publisher.post_batch_da(batch_id).await.unwrap();
        // Second call is idempotent
        publisher.post_batch_da(batch_id).await.unwrap();
    }
}
