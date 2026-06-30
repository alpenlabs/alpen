use alpen_ee_common::{
    AccessedStateRecord, Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus,
    EeAccountStateAtEpoch, ExecBlockRecord,
};
use strata_acct_types::Hash;
use strata_db_macros::gen_proxy;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};

use crate::{DbError, DbResult};

/// Database interface for EE node account state management.
#[gen_proxy(error = DbError, tracing_component = "storage:ee_node")]
pub(crate) trait EeNodeDb: Send + Sync + 'static {
    /// Stores EE account state for a given OL epoch commitment.
    fn store_ee_account_state(
        &self,
        ol_epoch: EpochCommitment,
        ee_account_state: EeAccountState,
    ) -> DbResult<()>;

    /// Rolls back EE account state to a specific epoch.
    fn rollback_ee_account_state(&self, to_epoch: u32) -> DbResult<()>;

    /// Retrieves the OL block ID for a given epoch number.
    fn get_ol_blockid(&self, epoch: u32) -> DbResult<Option<OLBlockId>>;

    /// Retrieves EE account state at a specific block ID.
    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtEpoch>>;

    /// Retrieves the most recent EE account state.
    fn best_ee_account_state(&self) -> DbResult<Option<EeAccountStateAtEpoch>>;

    /// Save block data and payload for a given block hash
    fn save_exec_block(&self, block: ExecBlockRecord, payload: Vec<u8>) -> DbResult<()>;

    /// Insert first block to local view of canonical finalized chain (ie. genesis block)
    fn init_finalized_chain(&self, hash: Hash) -> DbResult<()>;

    /// Extend local view of canonical chain up to and including the specified block hash.
    fn extend_finalized_chain(&self, new_tip: Hash) -> DbResult<()>;

    /// Revert local view of canonical chain to specified height
    fn revert_finalized_chain(&self, to_height: u64) -> DbResult<()>;

    /// Remove all block data below specified height
    fn prune_block_data(&self, to_height: u64) -> DbResult<()>;

    /// Get exec block for the highest blocknum available in the local view of canonical chain.
    fn best_finalized_block(&self) -> DbResult<Option<ExecBlockRecord>>;

    /// Get the finalized block at a specific height.
    fn get_finalized_block_at_height(&self, height: u64) -> DbResult<Option<ExecBlockRecord>>;

    /// Get height of block if it exists in local view of canonical chain.
    fn get_finalized_height(&self, hash: Hash) -> DbResult<Option<u64>>;

    /// Get all blocks in db with height > finalized height.
    /// The blockhashes should be ordered by incrementing height.
    fn get_unfinalized_blocks(&self) -> DbResult<Vec<Hash>>;

    /// Get block data for a specified block, if it exits.
    fn get_exec_block(&self, hash: Hash) -> DbResult<Option<ExecBlockRecord>>;

    /// Get block payload for a specified block, if it exists.
    fn get_block_payload(&self, hash: Hash) -> DbResult<Option<Vec<u8>>>;

    /// Delete a single block and its payload by hash.
    fn delete_exec_block(&self, hash: Hash) -> DbResult<()>;

    // Batch storage operations

    /// Save the genesis batch. Noop if any batches exist.
    fn save_genesis_batch(&self, batch: Batch) -> DbResult<()>;

    /// Save the next batch. Must extend the last batch present in storage.
    fn save_next_batch(&self, batch: Batch) -> DbResult<()>;

    /// Update an existing batch's status.
    fn update_batch_status(&self, batch_id: BatchId, status: BatchStatus) -> DbResult<()>;

    /// Remove all batches where idx > to_idx.
    fn revert_batches(&self, to_idx: u64) -> DbResult<()>;

    /// Get a batch by its id, if it exists.
    fn get_batch_by_id(&self, batch_id: BatchId) -> DbResult<Option<(Batch, BatchStatus)>>;

    /// Get a batch by its idx, if it exists.
    fn get_batch_by_idx(&self, idx: u64) -> DbResult<Option<(Batch, BatchStatus)>>;

    /// Get the batch with the highest idx, if it exists.
    fn get_latest_batch(&self) -> DbResult<Option<(Batch, BatchStatus)>>;

    // Chunk storage operations

    /// Save the next chunk.
    fn save_next_chunk(&self, chunk: Chunk) -> DbResult<()>;

    /// Update an existing chunk's status.
    fn update_chunk_status(&self, chunk_id: ChunkId, status: ChunkStatus) -> DbResult<()>;

    /// Remove all chunks where idx >= from_idx.
    fn revert_chunks_from(&self, from_idx: u64) -> DbResult<()>;

    /// Get a chunk by its id, if it exists.
    fn get_chunk_by_id(&self, chunk_id: ChunkId) -> DbResult<Option<(Chunk, ChunkStatus)>>;

    /// Get a chunk by its idx, if it exists.
    fn get_chunk_by_idx(&self, idx: u64) -> DbResult<Option<(Chunk, ChunkStatus)>>;

    /// Get the chunk with the highest idx, if it exists.
    fn get_latest_chunk(&self) -> DbResult<Option<(Chunk, ChunkStatus)>>;

    /// Get sealed chunks at or above `start_idx`, in ascending idx order.
    fn get_sealed_chunks(
        &self,
        start_idx: u64,
        limit: usize,
    ) -> DbResult<Vec<(Chunk, ChunkStatus)>>;

    /// Get proof-pending chunks at or above `start_idx`, in ascending idx order.
    fn get_proof_pending_chunks(
        &self,
        start_idx: u64,
        limit: usize,
    ) -> DbResult<Vec<(Chunk, ChunkStatus)>>;

    /// Set or update batch-chunk association.
    fn set_batch_chunks(&self, batch_id: BatchId, chunks: Vec<ChunkId>) -> DbResult<()>;

    /// Get the chunk-id list previously set for a batch.
    fn get_batch_chunks(&self, batch_id: BatchId) -> DbResult<Option<Vec<ChunkId>>>;

    // Per-block proof-witness operations
    //
    // Written by the EE block-production / import path at commit time
    // (depth-0), read by the chunk prover's input assembly.

    /// Store the per-block proof-witness for `block_id`. Overwrites if present.
    fn put_block_witness(&self, block_id: Hash, witness: Vec<u8>) -> DbResult<()>;

    /// Fetch the per-block proof-witness for `block_id`, if one exists.
    fn get_block_witness(&self, block_id: Hash) -> DbResult<Option<Vec<u8>>>;

    /// Delete a block's proof-witness. Idempotent.
    fn del_block_witness(&self, block_id: Hash) -> DbResult<()>;

    // Per-block accessed-state + content-addressed bytecode operations
    //
    // Written by the `AccessedStateGenerator` exex (phase 2) and read by
    // the chunk-builder to skip per-block re-execution at chunk-seal time.

    /// Store the accessed-state record for `block_id`. Overwrites if present.
    fn put_block_accessed_state(&self, block_id: Hash, record: AccessedStateRecord)
        -> DbResult<()>;

    /// Fetch the accessed-state record for `block_id`, if one exists.
    fn get_block_accessed_state(&self, block_id: Hash) -> DbResult<Option<AccessedStateRecord>>;

    /// Delete a block's accessed-state record. Idempotent.
    fn del_block_accessed_state(&self, block_id: Hash) -> DbResult<()>;

    /// Store a bytecode keyed by its code hash. Idempotent (content-addressed).
    fn put_bytecode(&self, code_hash: Hash, code: Vec<u8>) -> DbResult<()>;

    /// Fetch a bytecode by code hash, if present.
    fn get_bytecode(&self, code_hash: Hash) -> DbResult<Option<Vec<u8>>>;
}

pub(crate) mod ops {
    pub(crate) use super::EeNodeDbProxy as EeNodeOps;
}
