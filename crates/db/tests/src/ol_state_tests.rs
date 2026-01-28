//! OL state database tests using proptest strategies.

use strata_db_types::traits::OLStateDatabase;
use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::IStateAccessor;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};

// =============================================================================
// Proptest-based test functions
// =============================================================================

pub fn proptest_put_and_get_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLState,
) {
    db.put_toplevel_ol_state(commitment, state.clone())
        .expect("test: put toplevel");
    let retrieved_state = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get toplevel")
        .unwrap();
    assert_eq!(retrieved_state.cur_slot(), state.cur_slot());
}

pub fn proptest_get_latest_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment1: OLBlockCommitment,
    commitment2: OLBlockCommitment,
    state: OLState,
) {
    // Ensure commitment2 has higher slot for deterministic "latest"
    let (lower, higher) = if commitment1.slot() < commitment2.slot() {
        (commitment1, commitment2)
    } else if commitment1.slot() > commitment2.slot() {
        (commitment2, commitment1)
    } else {
        // Same slot, use lexicographic order of blkid
        if commitment1.blkid() < commitment2.blkid() {
            (commitment1, commitment2)
        } else {
            (commitment2, commitment1)
        }
    };

    db.put_toplevel_ol_state(lower, state.clone())
        .expect("test: put state 1");
    db.put_toplevel_ol_state(higher, state.clone())
        .expect("test: put state 2");

    let (latest_commitment, latest_state) = db
        .get_latest_toplevel_ol_state()
        .expect("test: get latest")
        .unwrap();
    assert_eq!(latest_commitment, higher);
    assert_eq!(latest_state.cur_slot(), state.cur_slot());
}

pub fn proptest_delete_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLState,
) {
    db.put_toplevel_ol_state(commitment, state)
        .expect("test: put toplevel");
    db.del_toplevel_ol_state(commitment)
        .expect("test: delete toplevel");
    let deleted = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get toplevel after delete");
    assert!(deleted.is_none());
}

pub fn proptest_put_and_get_write_batch(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLState,
) {
    let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
    db.put_ol_write_batch(commitment, wb.clone())
        .expect("test: put write batch");
    let retrieved_wb = db
        .get_ol_write_batch(commitment)
        .expect("test: get write batch")
        .unwrap();
    assert_eq!(
        retrieved_wb.global().get_cur_slot(),
        wb.global().get_cur_slot()
    );
}

pub fn proptest_delete_write_batch(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLState,
) {
    let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
    db.put_ol_write_batch(commitment, wb)
        .expect("test: put write batch");
    db.del_ol_write_batch(commitment)
        .expect("test: delete write batch");
    let deleted = db
        .get_ol_write_batch(commitment)
        .expect("test: get write batch after delete");
    assert!(deleted.is_none());
}

#[macro_export]
macro_rules! ol_state_db_tests {
    ($setup_expr:expr) => {
        proptest::proptest! {
            #[test]
            fn proptest_put_and_get_toplevel_ol_state(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_put_and_get_toplevel_ol_state(&db, commitment, state);
            }

            #[test]
            fn proptest_get_latest_toplevel_ol_state(
                commitment1 in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                commitment2 in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_get_latest_toplevel_ol_state(&db, commitment1, commitment2, state);
            }

            #[test]
            fn proptest_delete_toplevel_ol_state(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_delete_toplevel_ol_state(&db, commitment, state);
            }

            #[test]
            fn proptest_put_and_get_write_batch(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_put_and_get_write_batch(&db, commitment, state);
            }

            #[test]
            fn proptest_delete_write_batch(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_delete_write_batch(&db, commitment, state);
            }
        }
    };
}
