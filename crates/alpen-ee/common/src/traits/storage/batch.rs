use async_trait::async_trait;

use crate::{Batch, BatchId, BatchStatus, StorageError};

/// Storage for Batches.
///
/// A batch is a contiguous group of blocks. One batch corresponds to one Account
/// Update Operation sent to OL, and batch size should typically be limited by DA
/// size constraints.
///
/// [`BatchId`] is deterministic, based on ids of blocks covered, and so is unique
/// across forks and reorgs.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BatchStorage: Send + Sync {
    /// Save the genesis batch.
    ///
    /// If any batches exist in storage, this is a noop.
    async fn save_genesis_batch(&self, genesis_batch: Batch) -> Result<(), StorageError>;
    /// Save the next ee update.
    ///
    /// The entry must extend the last batch present in storage.
    async fn save_next_batch(&self, batch: Batch) -> Result<(), StorageError>;
    /// Update an existing ee update's status.
    async fn update_batch_status(
        &self,
        batch_id: BatchId,
        status: BatchStatus,
    ) -> Result<(), StorageError>;
    /// Remove all batches where idx > to_idx.
    async fn revert_batches(&self, to_idx: u64) -> Result<(), StorageError>;
    /// Get an ee update by its id, if it exists.
    async fn get_batch_by_id(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError>;
    /// Get an ee update by its idx, if it exists.
    async fn get_batch_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError>;
    /// Get the ee update with the highest idx, if it exists.
    async fn get_latest_batch(&self) -> Result<Option<(Batch, BatchStatus)>, StorageError>;
}

/// Macro to instantiate all BatchStorage tests for a given storage setup.
#[cfg(feature = "test-utils")]
#[macro_export]
macro_rules! batch_storage_tests {
    ($setup_expr:expr) => {
        #[tokio::test]
        async fn test_save_genesis_batch() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_save_genesis_batch(&storage).await;
        }

        #[tokio::test]
        async fn test_save_genesis_batch_idempotent() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_save_genesis_batch_idempotent(&storage).await;
        }

        #[tokio::test]
        async fn test_save_next_batch() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_save_next_batch(&storage).await;
        }

        #[tokio::test]
        async fn test_update_batch_status() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_update_batch_status(&storage).await;
        }

        #[tokio::test]
        async fn test_revert_batches() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_revert_batches(&storage).await;
        }

        #[tokio::test]
        async fn test_get_batch_by_id_and_idx() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_get_batch_by_id_and_idx(&storage).await;
        }

        #[tokio::test]
        async fn test_get_latest_batch() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_get_latest_batch(&storage).await;
        }

        #[tokio::test]
        async fn test_empty_batch_storage() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_empty_batch_storage(&storage).await;
        }
    };
}

#[cfg(feature = "test-utils")]
pub mod tests {
    use strata_acct_types::Hash;
    use strata_identifiers::Buf32;

    use super::*;
    use crate::{Batch, BatchStatus};

    fn create_test_hash(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1; // ensure non-zero
        bytes[31] = value;
        Hash::from(Buf32::new(bytes))
    }

    /// Create a genesis batch for testing.
    pub fn create_test_genesis_batch() -> Batch {
        let genesis_hash = create_test_hash(1);
        Batch::new_genesis_batch(genesis_hash, 0).unwrap()
    }

    /// Create a non-genesis batch for testing.
    pub fn create_test_batch(idx: u64, prev_block_val: u8, last_block_val: u8) -> Batch {
        let prev_block = create_test_hash(prev_block_val);
        let last_block = create_test_hash(last_block_val);
        Batch::new(idx, prev_block, last_block, idx * 10, Vec::new()).unwrap()
    }

    /// Test saving and retrieving genesis batch.
    pub async fn test_save_genesis_batch(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();

        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let (retrieved, status) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert_eq!(retrieved.idx(), genesis_batch.idx());
        assert_eq!(retrieved.last_block(), genesis_batch.last_block());
        assert!(matches!(status, BatchStatus::Genesis));

        let (retrieved, status) = storage
            .get_batch_by_id(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.idx(), genesis_batch.idx());
        assert!(matches!(status, BatchStatus::Genesis));
    }

    /// Test that save_genesis_batch is idempotent.
    pub async fn test_save_genesis_batch_idempotent(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();

        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let latest = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.0.idx(), 0);
    }

    /// Test saving sequential batches.
    pub async fn test_save_next_batch(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1.clone()).await.unwrap();

        let (retrieved, status) = storage.get_batch_by_idx(1).await.unwrap().unwrap();
        assert_eq!(retrieved.idx(), 1);
        assert!(matches!(status, BatchStatus::Sealed));
    }

    /// Test updating batch status.
    pub async fn test_update_batch_status(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let new_status = BatchStatus::DaPending { envelope_idx: 42 };
        storage
            .update_batch_status(genesis_batch.id(), new_status)
            .await
            .unwrap();

        let (_, status) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert!(matches!(
            status,
            BatchStatus::DaPending { envelope_idx: 42 }
        ));
    }

    /// Test reverting batches.
    pub async fn test_revert_batches(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage.save_genesis_batch(genesis_batch).await.unwrap();

        for i in 1..=5 {
            let batch = create_test_batch(i, i as u8, (i + 1) as u8);
            storage.save_next_batch(batch).await.unwrap();
        }

        storage.revert_batches(2).await.unwrap();

        assert!(storage.get_batch_by_idx(0).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(1).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(2).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(3).await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(4).await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(5).await.unwrap().is_none());

        let (latest, _) = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 2);
    }

    /// Test getting batch by id and idx.
    pub async fn test_get_batch_by_id_and_idx(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let (by_idx, _) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert_eq!(by_idx.idx(), 0);

        let (by_id, _) = storage
            .get_batch_by_id(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_id.idx(), 0);

        assert_eq!(by_idx.id(), by_id.id());
    }

    /// Test get_latest_batch returns None when empty, and highest idx when not empty.
    pub async fn test_get_latest_batch(storage: &impl BatchStorage) {
        assert!(storage.get_latest_batch().await.unwrap().is_none());

        let genesis_batch = create_test_genesis_batch();
        storage.save_genesis_batch(genesis_batch).await.unwrap();

        let (latest, _) = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 0);

        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1).await.unwrap();

        let (latest, _) = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 1);
    }

    /// Test empty storage behavior.
    pub async fn test_empty_batch_storage(storage: &impl BatchStorage) {
        assert!(storage.get_latest_batch().await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(0).await.unwrap().is_none());
        storage.revert_batches(0).await.unwrap();
    }
}
