use async_trait::async_trait;

use crate::{Chunk, ChunkId, ChunkStatus, EeUpdate, EeUpdateId, EeUpdateStatus, StorageError};

/// Storage for EeUpdates and Chunks
///
/// EeUpdates are units for DA and posting proven state updates to OL, typically limited by DA size
/// constraints.
/// Chunks are units for proving ee chain execution, typically limited by prover constraints.
///
/// An EeUpdate will consist of 1 or more chunks.
/// These chunks can be generated before or after updates, but the chunks must fit inside EeUpdate
/// boundaries.
/// ... | ----------- EeUpdate 1 ----------- | ------- EeUpdate 2 ------ | ...
/// ... | --- Chunk 1 ---- | --- Chunk 2 --- | -------- Chunk 3 -------- | ...
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait UpdateChunkStorage {
    /// Save the next ee update
    ///
    /// The entry must extend the last ee_update present in storage.
    async fn save_next_ee_update(&self, ee_update: EeUpdate) -> Result<(), StorageError>;
    /// Update an existing ee update's status
    async fn update_ee_update_status(
        &self,
        ee_update_id: EeUpdateId,
        status: EeUpdateStatus,
    ) -> Result<(), StorageError>;
    /// Remove all ee updates where idx > to_idx
    async fn revert_ee_update(&self, to_idx: u64) -> Result<(), StorageError>;
    /// Get an ee update by its id, if it exists
    async fn get_ee_update_by_id(
        &self,
        ee_update_id: EeUpdateId,
    ) -> Result<Option<EeUpdate>, StorageError>;
    /// Get an ee update by its idx, if it exists
    async fn get_ee_update_by_idx(&self, idx: u64) -> Result<Option<EeUpdate>, StorageError>;
    /// Get the ee update with the highest idx, if it exists.
    async fn get_latest_ee_update(&self) -> Result<Option<EeUpdate>, StorageError>;

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
    async fn get_chunk_by_id(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, StorageError>;
    /// Get a chunk by its idx, if it exists
    async fn get_chunk_by_idx(&self, idx: u64) -> Result<Option<Chunk>, StorageError>;
    /// Get the chunk with the highest id, if it exists.
    async fn get_latest_chunk(&self) -> Result<Option<Chunk>, StorageError>;
    /// Set or update EeUpdate and Chunk association
    async fn set_ee_update_chunks(
        &self,
        ee_update_id: EeUpdateId,
        chunks: Vec<ChunkId>,
    ) -> Result<(), StorageError>;
}
