use alpen_ee_common::{EeAccountStateAtEpoch, ExecBlockRecord};
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_storage_common::{inst_ops_ctx_shim_generic, inst_ops_generic};

use crate::{error::DbError, DbResult};

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
}

pub(crate) mod ops {
    use super::*;

    inst_ops_generic! {
        (<D: EeNodeDb> => EeNodeOps, DbError) {
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
        }
    }
}
