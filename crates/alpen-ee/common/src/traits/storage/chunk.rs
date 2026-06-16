//! Storage for Chunks and batch-chunk associations.
//!
//! Chunks are subsets of blocks in a batch, and units for proving EE chain
//! execution, typically limited by prover constraints.
//!
//! A batch will consist of 1 or more chunks. These chunks must fit inside
//! batch boundaries.
//!
//! ```text
//! ... | ------------ Batch 1 ------------- | --------- Batch 2 ------- | ...
//! ... | --- Chunk 1 ---- | --- Chunk 2 --- | -------- Chunk 3 -------- | ...
//! ```

use async_trait::async_trait;

use crate::{BatchId, Chunk, ChunkId, ChunkStatus, StorageError};

/// Storage for Chunks and batch-chunk associations.
///
/// [`ChunkId`] is deterministic, based on ids of blocks covered, and so is
/// unique across forks and reorgs.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait ChunkStorage: Send + Sync {
    /// Save the next chunk.
    ///
    /// The entry must extend the last chunk present in storage.
    async fn save_next_chunk(&self, chunk: Chunk) -> Result<(), StorageError>;

    /// Update an existing chunk's status.
    async fn update_chunk_status(
        &self,
        chunk_id: ChunkId,
        status: ChunkStatus,
    ) -> Result<(), StorageError>;

    /// Remove all chunks where idx >= from_idx.
    async fn revert_chunks_from(&self, from_idx: u64) -> Result<(), StorageError>;

    /// Get a chunk by its id, if it exists.
    async fn get_chunk_by_id(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;

    /// Get a chunk by its idx, if it exists.
    async fn get_chunk_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;

    /// Get the chunk with the highest idx, if it exists.
    async fn get_latest_chunk(&self) -> Result<Option<(Chunk, ChunkStatus)>, StorageError>;

    /// Set or update batch-chunk association.
    async fn set_batch_chunks(
        &self,
        batch_id: BatchId,
        chunks: Vec<ChunkId>,
    ) -> Result<(), StorageError>;

    /// Get the chunk-id list previously set for a batch.
    ///
    /// Returns `None` if [`set_batch_chunks`](ChunkStorage::set_batch_chunks)
    /// has never been called for `batch_id`. Used by the prover to fan out
    /// chunk-proof tasks.
    async fn get_batch_chunks(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<Vec<ChunkId>>, StorageError>;
}

/// Macro to instantiate all ChunkStorage tests for a given storage setup.
#[cfg(feature = "test-utils")]
#[macro_export]
macro_rules! chunk_storage_tests {
    ($setup_expr:expr) => {
        #[tokio::test]
        async fn test_save_next_chunk() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_save_next_chunk(&storage).await;
        }

        #[tokio::test]
        async fn test_update_chunk_status() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_update_chunk_status(&storage).await;
        }

        #[tokio::test]
        async fn test_revert_chunks() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_revert_chunks(&storage).await;
        }

        #[tokio::test]
        async fn test_set_batch_chunks() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_set_batch_chunks(&storage).await;
        }

        #[tokio::test]
        async fn test_empty_chunk_storage() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_empty_chunk_storage(&storage).await;
        }

        #[tokio::test]
        async fn test_chunk_revert_does_not_affect_batches() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_chunk_revert_does_not_affect_batches(&storage)
                .await;
        }

        #[tokio::test]
        async fn test_batch_chunks_isolation() {
            let storage = $setup_expr;
            $crate::chunk_storage_test_fns::test_batch_chunks_isolation(&storage).await;
        }
    };
}

#[cfg(feature = "test-utils")]
pub mod tests {
    use strata_acct_types::Hash;
    use strata_identifiers::Buf32;

    use super::*;
    use crate::{Batch, BatchStorage, Chunk, ChunkStatus, ProofId};

    fn create_test_hash(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1; // ensure non-zero
        bytes[31] = value;
        Hash::from(Buf32::new(bytes))
    }

    /// Create a chunk for testing.
    pub fn create_test_chunk(idx: u64, prev_block_val: u8, last_block_val: u8) -> Chunk {
        let prev_block = create_test_hash(prev_block_val);
        let last_block = create_test_hash(last_block_val);
        Chunk::new(idx, prev_block, last_block, 0, 0, Vec::new())
    }

    /// Create a genesis batch for testing (needed by isolation tests).
    fn create_test_genesis_batch() -> Batch {
        let genesis_hash = create_test_hash(1);
        Batch::new_genesis_batch(genesis_hash, 0).unwrap()
    }

    /// Create a non-genesis batch for testing (needed by isolation tests).
    fn create_test_batch(idx: u64, prev_block_val: u8, last_block_val: u8) -> Batch {
        let prev_block = create_test_hash(prev_block_val);
        let last_block = create_test_hash(last_block_val);
        Batch::new(idx, prev_block, last_block, idx * 10, Vec::new()).unwrap()
    }

    /// Test saving chunks.
    pub async fn test_save_next_chunk(storage: &(impl ChunkStorage + BatchStorage)) {
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
    pub async fn test_update_chunk_status(storage: &(impl ChunkStorage + BatchStorage)) {
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
    pub async fn test_revert_chunks(storage: &(impl ChunkStorage + BatchStorage)) {
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
    pub async fn test_set_batch_chunks(storage: &(impl ChunkStorage + BatchStorage)) {
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
            .set_batch_chunks(genesis_batch.id(), chunks.clone())
            .await
            .unwrap();

        // Verify round-trip
        let retrieved = storage
            .get_batch_chunks(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved, chunks);
    }

    /// Test empty chunk storage behavior.
    pub async fn test_empty_chunk_storage(storage: &(impl ChunkStorage + BatchStorage)) {
        // get_latest_chunk returns None
        assert!(storage.get_latest_chunk().await.unwrap().is_none());

        // get_chunk_by_idx returns None
        assert!(storage.get_chunk_by_idx(0).await.unwrap().is_none());

        // revert on empty storage should succeed
        storage.revert_chunks_from(0).await.unwrap();
    }

    /// Verify that reverting chunks does not affect batches and vice versa.
    pub async fn test_chunk_revert_does_not_affect_batches(
        storage: &(impl ChunkStorage + BatchStorage),
    ) {
        // Set up batches
        let genesis_batch = create_test_genesis_batch();
        storage.save_genesis_batch(genesis_batch).await.unwrap();
        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1).await.unwrap();

        // Set up chunks
        for i in 0..=2 {
            let chunk = create_test_chunk(i, i as u8, (i + 1) as u8);
            storage.save_next_chunk(chunk).await.unwrap();
        }

        // Revert chunks — batches should be unaffected
        storage.revert_chunks_from(1).await.unwrap();
        assert!(storage.get_chunk_by_idx(0).await.unwrap().is_some());
        assert!(storage.get_chunk_by_idx(1).await.unwrap().is_none());
        assert!(storage.get_batch_by_idx(0).await.unwrap().is_some());
        assert!(storage.get_batch_by_idx(1).await.unwrap().is_some());

        // Revert batches — remaining chunk should be unaffected
        storage.revert_batches(0).await.unwrap();
        assert!(storage.get_batch_by_idx(1).await.unwrap().is_none());
        assert!(storage.get_chunk_by_idx(0).await.unwrap().is_some());
    }

    /// Verify that batch-chunk associations for different batches are isolated.
    pub async fn test_batch_chunks_isolation(storage: &(impl ChunkStorage + BatchStorage)) {
        let genesis_batch = create_test_genesis_batch();
        storage
            .save_genesis_batch(genesis_batch.clone())
            .await
            .unwrap();
        let batch1 = create_test_batch(1, 1, 2);
        storage.save_next_batch(batch1.clone()).await.unwrap();

        let chunk0 = create_test_chunk(0, 0, 1);
        let chunk1 = create_test_chunk(1, 1, 2);
        let chunk2 = create_test_chunk(2, 2, 3);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage.save_next_chunk(chunk1.clone()).await.unwrap();
        storage.save_next_chunk(chunk2.clone()).await.unwrap();

        // Associate different chunks with different batches
        storage
            .set_batch_chunks(genesis_batch.id(), vec![chunk0.id()])
            .await
            .unwrap();
        storage
            .set_batch_chunks(batch1.id(), vec![chunk1.id(), chunk2.id()])
            .await
            .unwrap();

        // Verify isolation
        let genesis_chunks = storage
            .get_batch_chunks(genesis_batch.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(genesis_chunks, vec![chunk0.id()]);

        let batch1_chunks = storage
            .get_batch_chunks(batch1.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(batch1_chunks, vec![chunk1.id(), chunk2.id()]);

        // Unassociated batch returns None
        let fake_batch = create_test_batch(2, 2, 3);
        assert!(storage
            .get_batch_chunks(fake_batch.id())
            .await
            .unwrap()
            .is_none());
    }
}
