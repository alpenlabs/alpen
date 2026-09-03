//! OL state database tests using proptest strategies.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use strata_db_types::ol_state::OLStateDatabase;
use strata_identifiers::test_utils::{account_id_strategy, account_serial_strategy};
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types_v1::test_utils::ol_snark_account_state_strategy;
use strata_ol_state_types_v1::{OLAccountStateV1, OLAccountTypeStateV1, OLStateV1, WriteBatch};

// =============================================================================
// Proptest-based test functions
// =============================================================================

pub fn proptest_put_and_get_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLStateV1,
) {
    db.put_toplevel_ol_state(commitment, state.clone())
        .expect("test: put toplevel");
    let retrieved_state = db
        .get_toplevel_ol_state(commitment)
        .expect("test: get toplevel")
        .unwrap();
    assert_eq!(
        retrieved_state.global_state().get_cur_slot(),
        state.global_state().get_cur_slot()
    );
}

pub fn proptest_get_latest_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment1: OLBlockCommitment,
    commitment2: OLBlockCommitment,
    state: OLStateV1,
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
    assert_eq!(
        latest_state.global_state().get_cur_slot(),
        state.global_state().get_cur_slot()
    );
}

pub fn proptest_delete_toplevel_ol_state(
    db: &impl OLStateDatabase,
    commitment: OLBlockCommitment,
    state: OLStateV1,
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

pub fn proptest_put_and_get_write_batch(db: &impl OLStateDatabase, commitment: OLBlockCommitment) {
    let mut runner = TestRunner::deterministic();
    let created_id = account_id_strategy()
        .new_tree(&mut runner)
        .expect("generate created account ID")
        .current();
    let updated_id = loop {
        let id = account_id_strategy()
            .new_tree(&mut runner)
            .expect("generate updated account ID")
            .current();
        if id != created_id {
            break id;
        }
    };
    let created_serial = account_serial_strategy()
        .new_tree(&mut runner)
        .expect("generate created account serial")
        .current();
    let updated_serial = account_serial_strategy()
        .new_tree(&mut runner)
        .expect("generate updated account serial")
        .current();
    let created_snark_state = ol_snark_account_state_strategy()
        .new_tree(&mut runner)
        .expect("generate created snark account state")
        .current();
    let mut wb = WriteBatch::default();
    wb.ledger_mut().create_account_raw(
        created_id,
        OLAccountStateV1::new(
            created_serial,
            Default::default(),
            OLAccountTypeStateV1::Snark(created_snark_state),
        ),
        created_serial,
    );
    wb.ledger_mut().update_account(
        updated_id,
        OLAccountStateV1::new(
            updated_serial,
            Default::default(),
            OLAccountTypeStateV1::Empty,
        ),
    );
    db.put_ol_write_batch(commitment, wb.clone())
        .expect("test: put write batch");
    let retrieved_wb = db
        .get_ol_write_batch(commitment)
        .expect("test: get write batch")
        .unwrap();
    assert_eq!(
        retrieved_wb.global_writes().cur_slot,
        wb.global_writes().cur_slot,
    );
    assert_eq!(
        retrieved_wb.ledger().get_account(&created_id),
        wb.ledger().get_account(&created_id),
    );
    assert_eq!(
        retrieved_wb.ledger().get_account(&updated_id),
        wb.ledger().get_account(&updated_id),
    );
    assert_eq!(retrieved_wb.ledger().new_accounts(), &[created_id]);
}

pub fn proptest_delete_write_batch(db: &impl OLStateDatabase, commitment: OLBlockCommitment) {
    let wb = WriteBatch::default();
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
                state in strata_ol_state_types_v1::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_put_and_get_toplevel_ol_state(&db, commitment, state);
            }

            #[test]
            fn proptest_get_latest_toplevel_ol_state(
                commitment1 in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                commitment2 in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types_v1::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_get_latest_toplevel_ol_state(&db, commitment1, commitment2, state);
            }

            #[test]
            fn proptest_delete_toplevel_ol_state(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
                state in strata_ol_state_types_v1::test_utils::ol_state_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_delete_toplevel_ol_state(&db, commitment, state);
            }

            #[test]
            fn proptest_put_and_get_write_batch(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_put_and_get_write_batch(&db, commitment);
            }

            #[test]
            fn proptest_delete_write_batch(
                commitment in strata_identifiers::test_utils::ol_block_commitment_strategy(),
            ) {
                let db = $setup_expr;
                $crate::ol_state_tests::proptest_delete_write_batch(&db, commitment);
            }
        }
    };
}
