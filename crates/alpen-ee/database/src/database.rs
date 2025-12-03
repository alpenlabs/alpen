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

/// Macro to instantiate all EeNodeDb tests for a given database setup.
#[cfg(test)]
macro_rules! ee_node_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_store_and_get_ee_account_state() {
            let db = $setup_expr;
            $crate::database::tests::test_store_and_get_ee_account_state(&db);
        }

        #[test]
        fn test_sequential_slots() {
            let db = $setup_expr;
            $crate::database::tests::test_sequential_slots(&db);
        }

        #[test]
        fn test_null_block_rejected() {
            let db = $setup_expr;
            $crate::database::tests::test_null_block_rejected(&db);
        }

        #[test]
        fn test_rollback_ee_account_state() {
            let db = $setup_expr;
            $crate::database::tests::test_rollback_ee_account_state(&db);
        }

        #[test]
        fn test_empty_database() {
            let db = $setup_expr;
            $crate::database::tests::test_empty_database(&db);
        }

        #[test]
        fn test_rollback_empty_database() {
            let db = $setup_expr;
            $crate::database::tests::test_rollback_empty_database(&db);
        }

        #[test]
        fn test_sequential_writes_and_retrieval() {
            let db = $setup_expr;
            $crate::database::tests::test_sequential_writes_and_retrieval(&db);
        }
    };
}

#[cfg(test)]
pub(crate) use ee_node_db_tests;

#[cfg(test)]
pub(crate) mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_primitives::buf::Buf32;

    use super::*;

    fn create_test_block_id(value: u8) -> OLBlockId {
        OLBlockId::from(Buf32::new([value; 32]))
    }

    fn create_test_ee_account_state() -> EeAccountState {
        EeAccountState::new([0u8; 32], BitcoinAmount::ZERO, Vec::new(), Vec::new())
    }

    /// Test storing and retrieving EE account state.
    pub(crate) fn test_store_and_get_ee_account_state(db: &impl EeNodeDb) {
        // Create test data
        let slot = 100u64;
        let block_id = create_test_block_id(1);
        let ol_block = OLBlockCommitment::new(slot, block_id);
        let ee_account_state = create_test_ee_account_state();

        // Store the account state
        db.store_ee_account_state(ol_block, ee_account_state.clone())
            .unwrap();

        // Retrieve by block ID
        let retrieved = db.ee_account_state(block_id).unwrap().unwrap();
        assert_eq!(retrieved.ol_block(), &ol_block);
        assert_eq!(retrieved.ee_state(), &ee_account_state);

        // Retrieve by slot
        let retrieved_block_id = db.get_ol_blockid(slot).unwrap().unwrap();
        assert_eq!(retrieved_block_id, block_id);

        // Retrieve best state
        let best = db.best_ee_account_state().unwrap().unwrap();
        assert_eq!(best.ol_block(), &ol_block);
        assert_eq!(best.ee_state(), &ee_account_state);
    }

    /// Test sequential slot enforcement.
    pub(crate) fn test_sequential_slots(db: &impl EeNodeDb) {
        // First write should succeed at any slot
        let slot1 = 100u64;
        let block_id1 = create_test_block_id(1);
        let ol_block1 = OLBlockCommitment::new(slot1, block_id1);
        let ee_account_state1 = create_test_ee_account_state();

        db.store_ee_account_state(ol_block1, ee_account_state1.clone())
            .unwrap();

        // Next write must be at slot1 + 1
        let slot2 = slot1 + 1;
        let block_id2 = create_test_block_id(2);
        let ol_block2 = OLBlockCommitment::new(slot2, block_id2);
        let ee_account_state2 = create_test_ee_account_state();

        db.store_ee_account_state(ol_block2, ee_account_state2.clone())
            .unwrap();

        // Writing to a non-sequential slot should fail
        let slot_skip = slot2 + 2; // Skip slot2 + 1
        let block_id_skip = create_test_block_id(3);
        let ol_block_skip = OLBlockCommitment::new(slot_skip, block_id_skip);
        let ee_account_state_skip = create_test_ee_account_state();

        let result = db.store_ee_account_state(ol_block_skip, ee_account_state_skip);
        assert!(result.is_err());
    }

    /// Test null block rejection.
    pub(crate) fn test_null_block_rejected(db: &impl EeNodeDb) {
        let null_block = OLBlockCommitment::null();
        let ee_account_state = create_test_ee_account_state();

        let result = db.store_ee_account_state(null_block, ee_account_state);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::NullOlBlock));
    }

    /// Test rollback functionality.
    pub(crate) fn test_rollback_ee_account_state(db: &impl EeNodeDb) {
        // Create a sequence of states
        let slots = [100u64, 101, 102, 103, 104];
        let mut block_ids = Vec::new();

        for slot in slots {
            let block_id = create_test_block_id(slot as u8);
            block_ids.push(block_id);
            let ol_block = OLBlockCommitment::new(slot, block_id);
            let ee_account_state = create_test_ee_account_state();

            db.store_ee_account_state(ol_block, ee_account_state)
                .unwrap();
        }

        // Rollback to slot 101
        db.rollback_ee_account_state(101).unwrap();

        // Slots 100 and 101 should still exist
        assert!(db.get_ol_blockid(100).unwrap().is_some());
        assert!(db.get_ol_blockid(101).unwrap().is_some());

        // Slots 102, 103, 104 should be gone
        assert!(db.get_ol_blockid(102).unwrap().is_none());
        assert!(db.get_ol_blockid(103).unwrap().is_none());
        assert!(db.get_ol_blockid(104).unwrap().is_none());

        // Best state should be at slot 101
        let best = db.best_ee_account_state().unwrap().unwrap();
        assert_eq!(best.ol_block().slot(), 101);
    }

    /// Test empty database behavior.
    pub(crate) fn test_empty_database(db: &impl EeNodeDb) {
        // Best state should be None on empty db
        let best = db.best_ee_account_state().unwrap();
        assert!(best.is_none());

        // Getting non-existent block ID should return None
        let block_id = create_test_block_id(1);
        let state = db.ee_account_state(block_id).unwrap();
        assert!(state.is_none());

        // Getting non-existent slot should return None
        let slot_result = db.get_ol_blockid(999).unwrap();
        assert!(slot_result.is_none());
    }

    /// Test rollback on empty database.
    pub(crate) fn test_rollback_empty_database(db: &impl EeNodeDb) {
        // Rollback on empty db should succeed (no-op)
        let result = db.rollback_ee_account_state(100);
        assert!(result.is_ok());
    }

    /// Test multiple sequential writes and retrieval.
    pub(crate) fn test_sequential_writes_and_retrieval(db: &impl EeNodeDb) {
        let num_blocks = 10;
        let start_slot = 200u64;

        // Write multiple sequential blocks
        for i in 0..num_blocks {
            let slot = start_slot + i;
            let block_id = create_test_block_id(i as u8);
            let ol_block = OLBlockCommitment::new(slot, block_id);
            let ee_account_state = create_test_ee_account_state();

            db.store_ee_account_state(ol_block, ee_account_state)
                .unwrap();
        }

        // Verify all blocks can be retrieved
        for i in 0..num_blocks {
            let slot = start_slot + i;
            let expected_block_id = create_test_block_id(i as u8);

            let block_id = db.get_ol_blockid(slot).unwrap().unwrap();
            assert_eq!(block_id, expected_block_id);

            let state = db.ee_account_state(block_id).unwrap().unwrap();
            assert_eq!(state.ol_block().slot(), slot);
        }

        // Best state should be the last one
        let best = db.best_ee_account_state().unwrap().unwrap();
        assert_eq!(best.ol_block().slot(), start_slot + num_blocks - 1);
    }
}
