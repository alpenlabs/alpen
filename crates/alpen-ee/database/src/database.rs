use alpen_ee_common::{EeAccountStateAtBlock, ExecBlockRecord};
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_storage_common::{inst_ops_ctx_shim_generic, inst_ops_generic};

use crate::{error::DbError, DbResult};

#[expect(unused, reason = "wip")]
/// Database interface for EE node account state management.
pub(crate) trait EeNodeDb: Send + Sync + 'static {
    /// Stores EE account state for a given OL block commitment.
    fn store_ee_account_state(
        &self,
        ol_block: OLBlockCommitment,
        ee_account_state: EeAccountState,
    ) -> DbResult<()>;

    /// Rolls back EE account state to a specific slot.
    fn rollback_ee_account_state(&self, to_slot: u64) -> DbResult<()>;

    /// Retrieves the OL block ID for a given slot number.
    fn get_ol_blockid(&self, slot: u64) -> DbResult<Option<OLBlockId>>;

    /// Retrieves EE account state at a specific block ID.
    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtBlock>>;

    /// Retrieves the most recent EE account state.
    fn best_ee_account_state(&self) -> DbResult<Option<EeAccountStateAtBlock>>;

    /// Save block data and payload for a given block hash
    fn save_exec_block(&self, block: ExecBlockRecord, payload: Vec<u8>) -> DbResult<()>;

    /// Extend local view of canonical chain with specified block hash
    fn extend_finalized_chain(&self, hash: Hash) -> DbResult<()>;

    /// Revert local view of canonical chain to specified height
    fn revert_finalized_chain(&self, to_height: u64) -> DbResult<()>;

    /// Remove all block data below specified height
    fn prune_block_data(&self, to_height: u64) -> DbResult<()>;

    /// Get exec block for the highest blocknum available in the local view of canonical chain.
    fn best_finalized_block(&self) -> DbResult<Option<ExecBlockRecord>>;

    /// Get height of block if it exists in local view of canonical chain.
    fn get_finalized_height(&self, hash: Hash) -> DbResult<Option<u64>>;

    /// Get all blocks in db with height > finalized height.
    /// The blockhashes should be ordered by incrementing height.
    fn get_unfinalized_blocks(&self) -> DbResult<Vec<Hash>>;

    /// Get block data for a specified block, if it exits.
    fn get_exec_block(&self, hash: Hash) -> DbResult<Option<ExecBlockRecord>>;

    /// Get block payload for a specified block, if it exists.
    fn get_block_payload(&self, hash: Hash) -> DbResult<Option<Vec<u8>>>;
}

pub(crate) mod ops {
    use super::*;

    inst_ops_generic! {
        (<D: EeNodeDb> => EeNodeOps, DbError) {
            store_ee_account_state(ol_block: OLBlockCommitment, ee_account_state: EeAccountState) =>();
            rollback_ee_account_state(to_slot: u64) => ();
            get_ol_blockid(slot: u64) => Option<OLBlockId>;
            ee_account_state(block_id: OLBlockId) => Option<EeAccountStateAtBlock>;
            best_ee_account_state() => Option<EeAccountStateAtBlock>;

            save_exec_block(block: ExecBlockRecord, payload: Vec<u8>) => ();
            extend_finalized_chain(hash: Hash) => ();
            revert_finalized_chain(to_height: u64) => ();
            prune_block_data(to_height: u64) => ();
            best_finalized_block() => Option<ExecBlockRecord>;
            get_finalized_height(hash: Hash) => Option<u64>;
            get_unfinalized_blocks() => Vec<Hash>;
            get_exec_block(hash: Hash) => Option<ExecBlockRecord>;
            get_block_payload(hash: Hash) => Option<Vec<u8>>;
        }
    }
}
