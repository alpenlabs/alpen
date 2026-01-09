use strata_db_types::traits::OLStateDatabase;
use strata_identifiers::{OLBlockCommitment, OLBlockId, Slot};
use strata_ledger_types::IStateAccessor;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};

pub fn test_put_and_get_ol_state(db: &impl OLStateDatabase) {
    let state = OLState::new_genesis();
    let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

    // Test putting and getting state
    db.put_toplevel_ol_state(commitment, state.clone())
        .expect("test: put");
    let retrieved_state = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get")
        .unwrap();
    // Verify state was retrieved (can't compare directly as OLState doesn't implement PartialEq)
    assert_eq!(retrieved_state.cur_slot(), state.cur_slot());
}

pub fn test_get_latest_ol_state(db: &impl OLStateDatabase) {
    let state = OLState::new_genesis();
    let commitment1 = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());
    let commitment2 = OLBlockCommitment::new(Slot::from(1u64), OLBlockId::default());

    // Put two states with different slots
    db.put_toplevel_ol_state(commitment1, state.clone())
        .expect("test: put state 1");
    db.put_toplevel_ol_state(commitment2, state.clone())
        .expect("test: put state 2");

    // Latest should be the one with highest slot
    let (latest_commitment, latest_state) = db
        .get_latest_toplevel_ol_state()
        .expect("test: get latest")
        .unwrap();
    assert_eq!(latest_commitment, commitment2);
    assert_eq!(latest_state.cur_slot(), state.cur_slot());
}

pub fn test_delete_ol_state(db: &impl OLStateDatabase) {
    let state = OLState::new_genesis();
    let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

    // Put state
    db.put_toplevel_ol_state(commitment, state.clone())
        .expect("test: put");

    // Verify it exists
    let retrieved = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get")
        .unwrap();
    assert_eq!(retrieved.cur_slot(), state.cur_slot());

    // Delete it
    db.del_toplevel_ol_state(commitment).expect("test: delete");

    // Verify it's gone
    let deleted = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get after delete");
    assert!(deleted.is_none());
}

pub fn test_write_batch_operations(db: &impl OLStateDatabase) {
    let state = OLState::new_genesis();
    let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
    let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

    // Test putting and getting write batch
    db.put_ol_write_batch(commitment, wb.clone())
        .expect("test: put write batch");
    let retrieved_wb = db
        .get_ol_write_batch(commitment)
        .expect("test: get write batch")
        .unwrap();
    // Verify write batch was retrieved (can't compare directly as WriteBatch doesn't implement
    // PartialEq)
    assert_eq!(
        retrieved_wb.global().get_cur_slot(),
        wb.global().get_cur_slot()
    );

    // Test deleting write batch
    db.del_ol_write_batch(commitment)
        .expect("test: delete write batch");
    let deleted_wb = db
        .get_ol_write_batch(commitment)
        .expect("test: get after delete");
    assert!(deleted_wb.is_none());
}

#[macro_export]
macro_rules! ol_state_db_tests {
    ($setup_expr:expr) => {
        #[test]
        fn test_put_and_get_ol_state() {
            let db = $setup_expr;
            $crate::ol_state_tests::test_put_and_get_ol_state(&db);
        }

        #[test]
        fn test_get_latest_ol_state() {
            let db = $setup_expr;
            $crate::ol_state_tests::test_get_latest_ol_state(&db);
        }

        #[test]
        fn test_delete_ol_state() {
            let db = $setup_expr;
            $crate::ol_state_tests::test_delete_ol_state(&db);
        }

        #[test]
        fn test_write_batch_operations() {
            let db = $setup_expr;
            $crate::ol_state_tests::test_write_batch_operations(&db);
        }
    };
}
