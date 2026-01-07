use async_trait::async_trait;

use crate::{Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus, StorageError};

/// Storage for Batches and Chunks
///
/// A batch is a contiguous group of blocks. One batch corresponds to one Account Update Operation
/// sent to OL, and batch size should typically be limited by DA size constraints.
///
/// Chunks are subsets of blocks in a batch, and units for proving ee chain execution, typically
/// limited by prover constraints.
///
/// A batch will consist of 1 or more chunks.
/// These chunks can be generated before or after updates, but the chunks must fit inside batch
/// boundaries.
///
/// ... | ------------ Batch 1 ------------- | --------- Batch 2 ------- | ...
/// ... | --- Chunk 1 ---- | --- Chunk 2 --- | -------- Chunk 3 -------- | ...
///
/// BatchId and ChunkId are deterministic, based on ids of blocks covered, and so are unique across
/// forks and reorgs.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BatchStorage: Send + Sync {
    /// Save the genesis batch.
    ///
    /// If any batches exist in storage, this is a noop.
    async fn save_genesis_batch(&self, genesis_batch: Batch) -> Result<(), StorageError>;
    /// Save the next ee update
    ///
    /// The entry must extend the last batch present in storage.
    async fn save_next_batch(&self, batch: Batch) -> Result<(), StorageError>;
    /// Update an existing ee update's status
    async fn update_batch_status(
        &self,
        batch_id: BatchId,
        status: BatchStatus,
    ) -> Result<(), StorageError>;
    /// Remove all batches where idx > to_idx
    async fn revert_batches(&self, to_idx: u64) -> Result<(), StorageError>;
    /// Get an ee update by its id, if it exists
    async fn get_batch_by_id(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError>;
    /// Get an ee update by its idx, if it exists
    async fn get_batch_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError>;
    /// Get the ee update with the highest idx, if it exists.
    async fn get_latest_batch(&self) -> Result<Option<(Batch, BatchStatus)>, StorageError>;

    /// Save the next chunk
    ///
    /// The entry must extend the last chunk present in storage.
    async fn save_next_chunk(&self, chunk: Chunk) -> Result<(), StorageError>;
    /// Update an existing chunk's status
    async fn update_chunk_status(
        &self,
        chunk_id: ChunkId,
        status: ChunkStatus,
    ) -> Result<(), StorageError>;
    /// Remove all chunks where idx >= from_idx
    async fn revert_chunks_from(&self, from_idx: u64) -> Result<(), StorageError>;
    /// Get a chunk by its id, if it exists
    async fn get_chunk_by_id(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;
    /// Get a chunk by its idx, if it exists
    async fn get_chunk_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;
    /// Get the chunk with the highest id, if it exists.
    async fn get_latest_chunk(&self) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;
    /// Set or update Batch and Chunk association
    async fn set_batch_chunks(
        &self,
        batch_id: BatchId,
        chunks: Vec<ChunkId>,
    ) -> Result<(), StorageError>;
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
        async fn test_save_next_chunk() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_save_next_chunk(&storage).await;
        }

        #[tokio::test]
        async fn test_update_chunk_status() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_update_chunk_status(&storage).await;
        }

        #[tokio::test]
        async fn test_revert_chunks() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_revert_chunks(&storage).await;
        }

        #[tokio::test]
        async fn test_set_batch_chunks() {
            let storage = $setup_expr;
            $crate::batch_storage_test_fns::test_set_batch_chunks(&storage).await;
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
    use crate::{Batch, BatchStatus, Chunk, ChunkStatus, ProofId};

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

    /// Create a chunk for testing.
    pub fn create_test_chunk(idx: u64, prev_block_val: u8, last_block_val: u8) -> Chunk {
        let prev_block = create_test_hash(prev_block_val);
        let last_block = create_test_hash(last_block_val);
        Chunk::new(idx, prev_block, last_block, Vec::new())
    }

    /// Test saving and retrieving genesis batch.
    pub async fn test_save_genesis_batch(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();

        // Save genesis batch
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        // Retrieve by idx
        let (retrieved, status) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert_eq!(retrieved.idx(), genesis_batch.idx());
        assert_eq!(retrieved.last_block(), genesis_batch.last_block());
        assert!(matches!(status, BatchStatus::Sealed));

        // Retrieve by id
        let (retrieved, _) = storage
            .get_batch_by_id(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.idx(), genesis_batch.idx());
    }

    /// Test that save_genesis_batch is idempotent.
    pub async fn test_save_genesis_batch_idempotent(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();

        // Save genesis batch twice
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        // Should only have one batch
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

        // Save next batch (idx=1, prev_block = genesis last_block)
        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1.clone()).await.unwrap();

        // Verify it was saved
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

        // Update status
        let new_status = BatchStatus::DaPending { txns: Vec::new() };
        storage
            .update_batch_status(genesis_batch.id(), new_status)
            .await
            .unwrap();

        // Verify status was updated
        let (_, status) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert!(matches!(status, BatchStatus::DaPending { .. }));
    }

    /// Test reverting batches.
    pub async fn test_revert_batches(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage.save_genesis_batch(genesis_batch).await.unwrap();

        // Save more batches
        for i in 1..=5 {
            let batch = create_test_batch(i, i as u8, (i + 1) as u8);
            storage.save_next_batch(batch).await.unwrap();
        }

        // Revert to idx 2 (keep genesis, 1, 2)
        storage.revert_batches(2).await.unwrap();

        // Verify batches 3, 4, 5 are gone
        assert!(storage.get_batch_by_idx(0).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(1).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(2).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(3).await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(4).await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(5).await.unwrap().is_none());

        // Latest should be at idx 2
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

        // Get by idx
        let (by_idx, _) = storage.get_batch_by_idx(0).await.unwrap().unwrap();
        assert_eq!(by_idx.idx(), 0);

        // Get by id
        let (by_id, _) = storage
            .get_batch_by_id(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_id.idx(), 0);

        // Both should return the same batch
        assert_eq!(by_idx.id(), by_id.id());
    }

    /// Test get_latest_batch returns None when empty, and highest idx when not empty.
    pub async fn test_get_latest_batch(storage: &impl BatchStorage) {
        // Empty storage returns None
        assert!(storage.get_latest_batch().await.unwrap().is_none());

        // Add genesis
        let genesis_batch = create_test_genesis_batch();
        storage.save_genesis_batch(genesis_batch).await.unwrap();

        // Latest should be genesis (idx 0)
        let (latest, _) = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 0);

        // Add more batches
        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1).await.unwrap();

        let (latest, _) = storage.get_latest_batch().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 1);
    }

    /// Test saving chunks.
    pub async fn test_save_next_chunk(storage: &impl BatchStorage) {
        let chunk0 = create_test_chunk(0, 0, 1);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();

        // Retrieve by idx
        let (retrieved, status) = storage.get_chunk_by_idx(0).await.unwrap().unwrap();
        assert_eq!(retrieved.idx(), 0);
        assert!(matches!(status, ChunkStatus::ProvingNotStarted));

        // Retrieve by id
        let (retrieved, _) = storage.get_chunk_by_id(chunk0.id()).await.unwrap().unwrap();
        assert_eq!(retrieved.idx(), 0);
    }

    /// Test updating chunk status.
    pub async fn test_update_chunk_status(storage: &impl BatchStorage) {
        let chunk0 = create_test_chunk(0, 0, 1);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();

        // Update status
        let proof_id = ProofId::from(Buf32::new([1u8; 32]));
        let new_status = ChunkStatus::ProofReady(proof_id);
        storage
            .update_chunk_status(chunk0.id(), new_status)
            .await
            .unwrap();

        // Verify status was updated
        let (_, status) = storage.get_chunk_by_idx(0).await.unwrap().unwrap();
        assert!(matches!(status, ChunkStatus::ProofReady(_)));
    }

    /// Test reverting chunks.
    pub async fn test_revert_chunks(storage: &impl BatchStorage) {
        // Save chunks 0, 1, 2, 3, 4
        for i in 0..=4 {
            let chunk = create_test_chunk(i, i as u8, (i + 1) as u8);
            storage.save_next_chunk(chunk).await.unwrap();
        }

        // Revert from idx 2 (remove chunks 2, 3, 4)
        storage.revert_chunks_from(2).await.unwrap();

        // Verify chunks 2, 3, 4 are gone
        assert!(storage.get_chunk_by_idx(0).await.unwrap().is_some());
        assert!(storage.get_chunk_by_idx(1).await.unwrap().is_some());
        assert!(storage.get_chunk_by_idx(2).await.unwrap().is_none());
        assert!(storage.get_chunk_by_idx(3).await.unwrap().is_none());
        assert!(storage.get_chunk_by_idx(4).await.unwrap().is_none());

        // Latest should be at idx 1
        let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
        assert_eq!(latest.idx(), 1);
    }

    /// Test batch-chunk association.
    pub async fn test_set_batch_chunks(storage: &impl BatchStorage) {
        let genesis_batch = create_test_genesis_batch();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();

        let chunk0 = create_test_chunk(0, 0, 1);
        let chunk1 = create_test_chunk(1, 1, 2);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage.save_next_chunk(chunk1.clone()).await.unwrap();

        // Set batch-chunk association
        let chunks = vec![chunk0.id(), chunk1.id()];
        storage
            .set_batch_chunks(genesis_batch.id(), chunks)
            .await
            .unwrap();

        // The association is set - we can't verify it directly without a get method,
        // but we can verify the operation doesn't fail
    }

    /// Test empty storage behavior.
    pub async fn test_empty_batch_storage(storage: &impl BatchStorage) {
        // get_latest_batch returns None
        assert!(storage.get_latest_batch().await.unwrap().is_none());

        // get_latest_chunk returns None
        assert!(storage.get_latest_chunk().await.unwrap().is_none());

        // get_batch_by_idx returns None
        assert!(storage.get_batch_by_idx(0).await.unwrap().is_none());

        // get_chunk_by_idx returns None
        assert!(storage.get_chunk_by_idx(0).await.unwrap().is_none());

        // revert on empty storage should succeed
        storage.revert_batches(0).await.unwrap();
        storage.revert_chunks_from(0).await.unwrap();
    }
}
