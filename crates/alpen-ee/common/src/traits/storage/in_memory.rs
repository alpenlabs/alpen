//! In-memory implementation of BatchStorage for testing.

use std::{
    collections::{BTreeMap, HashMap},
    sync::RwLock,
};

use async_trait::async_trait;

use crate::{Batch, BatchId, BatchStatus, BatchStorage, Chunk, ChunkId, ChunkStatus, StorageError};

/// In-memory storage for batches and chunks.
#[derive(Debug, Default)]
pub struct InMemoryStorage {
    batches: RwLock<BTreeMap<u64, (Batch, BatchStatus)>>,
    batch_id_to_idx: RwLock<HashMap<BatchId, u64>>,
    chunks: RwLock<BTreeMap<u64, (Chunk, ChunkStatus)>>,
    chunk_id_to_idx: RwLock<HashMap<ChunkId, u64>>,
    batch_chunks: RwLock<HashMap<BatchId, Vec<ChunkId>>>,
}

impl InMemoryStorage {
    /// Create a new empty in-memory storage.
    pub fn new_empty() -> Self {
        Self::default()
    }

    /// Create storage pre-populated with a genesis batch.
    pub fn with_genesis(genesis_batch: Batch) -> Self {
        let storage = Self::new_empty();
        let mut batches = storage.batches.write().unwrap();
        let mut id_to_idx = storage.batch_id_to_idx.write().unwrap();

        id_to_idx.insert(genesis_batch.id(), genesis_batch.idx());
        batches.insert(genesis_batch.idx(), (genesis_batch, BatchStatus::Sealed));

        drop(batches);
        drop(id_to_idx);
        storage
    }
}

#[async_trait]
impl BatchStorage for InMemoryStorage {
    async fn save_genesis_batch(&self, genesis_batch: Batch) -> Result<(), StorageError> {
        let mut batches = self.batches.write().unwrap();
        let mut id_to_idx = self.batch_id_to_idx.write().unwrap();

        // Idempotent - if any batches exist, this is a noop
        if !batches.is_empty() {
            return Ok(());
        }

        id_to_idx.insert(genesis_batch.id(), genesis_batch.idx());
        batches.insert(genesis_batch.idx(), (genesis_batch, BatchStatus::Genesis));
        Ok(())
    }

    async fn save_next_batch(&self, batch: Batch) -> Result<(), StorageError> {
        let mut batches = self.batches.write().unwrap();
        let mut id_to_idx = self.batch_id_to_idx.write().unwrap();

        // Verify it extends the last batch
        if let Some((&last_idx, _)) = batches.last_key_value() {
            if batch.idx() != last_idx + 1 {
                return Err(StorageError::MissingSlot {
                    attempted_slot: batch.idx(),
                    last_slot: last_idx,
                });
            }
        }

        id_to_idx.insert(batch.id(), batch.idx());
        batches.insert(batch.idx(), (batch, BatchStatus::Sealed));
        Ok(())
    }

    async fn update_batch_status(
        &self,
        batch_id: BatchId,
        status: BatchStatus,
    ) -> Result<(), StorageError> {
        let id_to_idx = self.batch_id_to_idx.read().unwrap();
        let idx = id_to_idx.get(&batch_id).copied();
        drop(id_to_idx);

        if let Some(idx) = idx {
            let mut batches = self.batches.write().unwrap();
            if let Some((batch, _)) = batches.remove(&idx) {
                batches.insert(idx, (batch, status));
            }
        }
        Ok(())
    }

    async fn revert_batches(&self, to_idx: u64) -> Result<(), StorageError> {
        let mut batches = self.batches.write().unwrap();
        let mut id_to_idx = self.batch_id_to_idx.write().unwrap();

        // Remove all batches where idx > to_idx
        let to_remove: Vec<u64> = batches
            .keys()
            .filter(|&&idx| idx > to_idx)
            .copied()
            .collect();

        for idx in to_remove {
            if let Some((batch, _)) = batches.remove(&idx) {
                id_to_idx.remove(&batch.id());
            }
        }
        Ok(())
    }

    async fn get_batch_by_id(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError> {
        let id_to_idx = self.batch_id_to_idx.read().unwrap();
        let idx = id_to_idx.get(&batch_id).copied();
        drop(id_to_idx);

        if let Some(idx) = idx {
            let batches = self.batches.read().unwrap();
            Ok(batches.get(&idx).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_batch_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Batch, BatchStatus)>, StorageError> {
        let batches = self.batches.read().unwrap();
        Ok(batches.get(&idx).cloned())
    }

    async fn get_latest_batch(&self) -> Result<Option<(Batch, BatchStatus)>, StorageError> {
        let batches = self.batches.read().unwrap();
        Ok(batches.last_key_value().map(|(_, v)| v.clone()))
    }

    async fn save_next_chunk(&self, chunk: Chunk) -> Result<(), StorageError> {
        let mut chunks = self.chunks.write().unwrap();
        let mut id_to_idx = self.chunk_id_to_idx.write().unwrap();

        // Verify it extends the last chunk (or is first)
        if let Some((&last_idx, _)) = chunks.last_key_value() {
            if chunk.idx() != last_idx + 1 {
                return Err(StorageError::MissingSlot {
                    attempted_slot: chunk.idx(),
                    last_slot: last_idx,
                });
            }
        }

        id_to_idx.insert(chunk.id(), chunk.idx());
        chunks.insert(chunk.idx(), (chunk, ChunkStatus::ProvingNotStarted));
        Ok(())
    }

    async fn update_chunk_status(
        &self,
        chunk_id: ChunkId,
        status: ChunkStatus,
    ) -> Result<(), StorageError> {
        let id_to_idx = self.chunk_id_to_idx.read().unwrap();
        let idx = id_to_idx.get(&chunk_id).copied();
        drop(id_to_idx);

        if let Some(idx) = idx {
            let mut chunks = self.chunks.write().unwrap();
            if let Some((chunk, _)) = chunks.remove(&idx) {
                chunks.insert(idx, (chunk, status));
            }
        }
        Ok(())
    }

    async fn revert_chunks_from(&self, from_idx: u64) -> Result<(), StorageError> {
        let mut chunks = self.chunks.write().unwrap();
        let mut id_to_idx = self.chunk_id_to_idx.write().unwrap();

        // Remove all chunks where idx >= from_idx
        let to_remove: Vec<u64> = chunks
            .keys()
            .filter(|&&idx| idx >= from_idx)
            .copied()
            .collect();

        for idx in to_remove {
            if let Some((chunk, _)) = chunks.remove(&idx) {
                id_to_idx.remove(&chunk.id());
            }
        }
        Ok(())
    }

    async fn get_chunk_by_id(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError> {
        let id_to_idx = self.chunk_id_to_idx.read().unwrap();
        let idx = id_to_idx.get(&chunk_id).copied();
        drop(id_to_idx);

        if let Some(idx) = idx {
            let chunks = self.chunks.read().unwrap();
            Ok(chunks.get(&idx).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_chunk_by_idx(
        &self,
        idx: u64,
    ) -> Result<Option<(Chunk, ChunkStatus)>, StorageError> {
        let chunks = self.chunks.read().unwrap();
        Ok(chunks.get(&idx).cloned())
    }

    async fn get_latest_chunk(&self) -> Result<Option<(Chunk, ChunkStatus)>, StorageError> {
        let chunks = self.chunks.read().unwrap();
        Ok(chunks.last_key_value().map(|(_, v)| v.clone()))
    }

    async fn set_batch_chunks(
        &self,
        batch_id: BatchId,
        chunks: Vec<ChunkId>,
    ) -> Result<(), StorageError> {
        let mut batch_chunks = self.batch_chunks.write().unwrap();
        batch_chunks.insert(batch_id, chunks);
        Ok(())
    }
}

#[cfg(all(test, feature = "test-utils"))]
mod in_memory_tests {
    use super::InMemoryStorage;
    use crate::batch_storage_tests;

    batch_storage_tests!(InMemoryStorage::new_empty());
}
