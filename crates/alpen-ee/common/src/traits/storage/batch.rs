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
/// An Batch will consist of 1 or more chunks.
/// These chunks can be generated before or after updates, but the chunks must fit inside Batch
/// boundaries.
/// ... | ----------- Batch 1 ----------- | ------- Batch 2 ------ | ...
/// ... | --- Chunk 1 ---- | --- Chunk 2 --- | -------- Chunk 3 -------- | ...
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BatchStorage {
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
    /// Remove all ee updates where idx > to_idx
    async fn revert_batch(&self, to_idx: u64) -> Result<(), StorageError>;
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
    /// Remove all chunks where idx > to_idx
    async fn revert_chunks(&self, to_idx: u64) -> Result<(), StorageError>;
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
