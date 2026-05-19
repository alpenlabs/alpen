//! Pre-computed chunk witness storage.
//!
//! Producer: the batch builder, at chunk-seal time (`seal_batch` in
//! `crates/alpen-ee/sequencer/src/batch_builder/task.rs`).
//! Consumer: `ChunkSpec::fetch_input` in the chunk prover.
//!
//! Sealing-time pre-computation replaces the on-demand
//! `RangeWitnessExtractor` invocation that previously ran at proof-request
//! time and OOMed as historical depth grew.

use async_trait::async_trait;

use crate::{ChunkId, ChunkWitnessRecord, StorageError};

/// Persistent store for pre-computed chunk witnesses.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait ChunkWitnessStore: Send + Sync {
    /// Insert or overwrite the witness record for `chunk_id`.
    async fn put_chunk_witness(
        &self,
        chunk_id: ChunkId,
        witness: ChunkWitnessRecord,
    ) -> Result<(), StorageError>;

    /// Fetch the witness record for `chunk_id`, if one has been persisted.
    async fn get_chunk_witness(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Option<ChunkWitnessRecord>, StorageError>;

    /// Remove the witness record for `chunk_id`. Idempotent — succeeds
    /// whether or not the record was present.
    async fn del_chunk_witness(&self, chunk_id: ChunkId) -> Result<(), StorageError>;
}
