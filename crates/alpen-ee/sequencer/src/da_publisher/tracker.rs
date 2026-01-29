//! BatchId to blob_hash tracking for DA publications.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use alpen_ee_common::BatchId;

/// Tracks the mapping between BatchId and blob_hash for DA publications.
///
/// This is used to correlate batch lifecycle operations with chunked blob
/// publications. The mapping is stored in memory since it can be reconstructed
/// from the database if needed.
#[derive(Debug, Clone, Default)]
pub struct DaTracker {
    inner: Arc<RwLock<DaTrackerInner>>,
}

#[derive(Debug, Default)]
struct DaTrackerInner {
    /// BatchId → blob_hash mapping.
    batch_to_blob: HashMap<BatchId, [u8; 32]>,
    /// blob_hash → BatchId reverse mapping.
    blob_to_batch: HashMap<[u8; 32], BatchId>,
}

impl DaTracker {
    /// Creates a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a batch → blob mapping.
    pub fn record_submission(&self, batch_id: BatchId, blob_hash: [u8; 32]) {
        let mut inner = self.inner.write().expect("lock poisoned");
        inner.batch_to_blob.insert(batch_id, blob_hash);
        inner.blob_to_batch.insert(blob_hash, batch_id);
    }

    /// Gets the blob_hash for a batch.
    pub fn get_blob_hash(&self, batch_id: &BatchId) -> Option<[u8; 32]> {
        let inner = self.inner.read().expect("lock poisoned");
        inner.batch_to_blob.get(batch_id).copied()
    }

    /// Gets the BatchId for a blob.
    pub fn get_batch_id(&self, blob_hash: &[u8; 32]) -> Option<BatchId> {
        let inner = self.inner.read().expect("lock poisoned");
        inner.blob_to_batch.get(blob_hash).copied()
    }

    /// Removes a batch mapping.
    pub fn remove_batch(&self, batch_id: &BatchId) {
        let mut inner = self.inner.write().expect("lock poisoned");
        if let Some(blob_hash) = inner.batch_to_blob.remove(batch_id) {
            inner.blob_to_batch.remove(&blob_hash);
        }
    }

    /// Clears all mappings.
    pub fn clear(&self) {
        let mut inner = self.inner.write().expect("lock poisoned");
        inner.batch_to_blob.clear();
        inner.blob_to_batch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_acct_types::Hash;

    fn make_batch_id(prev: u8, last: u8) -> BatchId {
        BatchId::from_parts(Hash::from([prev; 32]), Hash::from([last; 32]))
    }

    #[test]
    fn test_record_and_get() {
        let tracker = DaTracker::new();
        let batch_id = make_batch_id(1, 2);
        let blob_hash = [0x42; 32];

        tracker.record_submission(batch_id, blob_hash);

        assert_eq!(tracker.get_blob_hash(&batch_id), Some(blob_hash));
        assert_eq!(tracker.get_batch_id(&blob_hash), Some(batch_id));
    }

    #[test]
    fn test_remove_batch() {
        let tracker = DaTracker::new();
        let batch_id = make_batch_id(1, 2);
        let blob_hash = [0x42; 32];

        tracker.record_submission(batch_id, blob_hash);
        tracker.remove_batch(&batch_id);

        assert_eq!(tracker.get_blob_hash(&batch_id), None);
        assert_eq!(tracker.get_batch_id(&blob_hash), None);
    }

    #[test]
    fn test_not_found() {
        let tracker = DaTracker::new();
        let batch_id = make_batch_id(1, 2);
        let blob_hash = [0x42; 32];

        assert_eq!(tracker.get_blob_hash(&batch_id), None);
        assert_eq!(tracker.get_batch_id(&blob_hash), None);
    }
}
