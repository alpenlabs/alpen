use alpen_ee_common::{
    Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus, EeAccountStateAtEpoch,
    ExecBlockRecord,
};
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_storage_common::inst_ops_generic;

use crate::{error::DbError, instrumentation::components, DbResult};

/// Database interface for EE node account state management.
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

    /// Extend local view of canonical chain with specified block hash
    fn extend_finalized_chain(&self, hash: Hash) -> DbResult<()>;

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

    /// Set or update batch-chunk association.
    fn set_batch_chunks(&self, batch_id: BatchId, chunks: Vec<ChunkId>) -> DbResult<()>;
}

pub(crate) mod ops {
    use super::*;

    inst_ops_generic! {
        (<D: EeNodeDb> => EeNodeOps, DbError, component = components::STORAGE_EE_NODE) {
            store_ee_account_state(ol_epoch: EpochCommitment, ee_account_state: EeAccountState) =>();
            rollback_ee_account_state(to_epoch: u32) => ();
            get_ol_blockid(epoch: u32) => Option<OLBlockId>;
            ee_account_state(block_id: OLBlockId) => Option<EeAccountStateAtEpoch>;
            best_ee_account_state() => Option<EeAccountStateAtEpoch>;

            save_exec_block(block: ExecBlockRecord, payload: Vec<u8>) => ();
            init_finalized_chain(hash: Hash) => ();
            extend_finalized_chain(hash: Hash) => ();
            revert_finalized_chain(to_height: u64) => ();
            prune_block_data(to_height: u64) => ();
            best_finalized_block() => Option<ExecBlockRecord>;
            get_finalized_block_at_height(height: u64) => Option<ExecBlockRecord>;
            get_finalized_height(hash: Hash) => Option<u64>;
            get_unfinalized_blocks() => Vec<Hash>;
            get_exec_block(hash: Hash) => Option<ExecBlockRecord>;
            get_block_payload(hash: Hash) => Option<Vec<u8>>;
            delete_exec_block(hash: Hash) => ();

            // Batch operations
            save_genesis_batch(batch: Batch) => ();
            save_next_batch(batch: Batch) => ();
            update_batch_status(batch_id: BatchId, status: BatchStatus) => ();
            revert_batches(to_idx: u64) => ();
            get_batch_by_id(batch_id: BatchId) => Option<(Batch, BatchStatus)>;
            get_batch_by_idx(idx: u64) => Option<(Batch, BatchStatus)>;
            get_latest_batch() => Option<(Batch, BatchStatus)>;

            // Chunk operations
            save_next_chunk(chunk: Chunk) => ();
            update_chunk_status(chunk_id: ChunkId, status: ChunkStatus) => ();
            revert_chunks_from(from_idx: u64) => ();
            get_chunk_by_id(chunk_id: ChunkId) => Option<(Chunk, ChunkStatus)>;
            get_chunk_by_idx(idx: u64) => Option<(Chunk, ChunkStatus)>;
            get_latest_chunk() => Option<(Chunk, ChunkStatus)>;
            set_batch_chunks(batch_id: BatchId, chunks: Vec<ChunkId>) => ();
        }
    }
}
