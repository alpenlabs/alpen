//! Tests for transaction slot constraints.

use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxConstraintsV1, TxProofsV1};

use crate::errors::ExecError;
use crate::test_utils::*;

fn fixture_with_gam_target() -> OLStfFixture {
    OLStfFixture::builder()
        .with_genesis_empty_account(make_account_id(TEST_RECIPIENT_ID))
        .execute_genesis()
}

fn constrained_gam_tx(min_slot: Option<u64>, max_slot: Option<u64>) -> OLTransactionV1 {
    let target_acct_id = make_account_id(TEST_RECIPIENT_ID);
    OLTransactionV1::new(
        OLTransactionDataV1::from_gam_bytes(target_acct_id, vec![])
            .expect("message payload bytes must fit within SSZ max length")
            .with_constraints(TxConstraintsV1::new(min_slot, max_slot)),
        TxProofsV1::new_empty(),
    )
}

#[test]
fn test_tx_constraints_allow_min_slot_boundary() {
    let mut fixture = fixture_with_gam_target();

    fixture
        .child_block()
        .with_slot(1)
        .with_tx(constrained_gam_tx(Some(1), None))
        .execute();
}

#[test]
fn test_tx_constraints_reject_before_min_slot() {
    let mut fixture = fixture_with_gam_target();

    let err = fixture
        .child_block()
        .with_slot(1)
        .with_tx(constrained_gam_tx(Some(2), None))
        .execute_err();

    match err.into_base() {
        ExecError::TransactionNotMature(min_slot, current_slot) => {
            assert_eq!(min_slot, 2);
            assert_eq!(current_slot, 1);
        }
        err => panic!("Expected TransactionNotMature, got: {err:?}"),
    }
}

#[test]
fn test_tx_constraints_allow_max_slot_boundary() {
    let mut fixture = fixture_with_gam_target();

    fixture
        .child_block()
        .with_slot(1)
        .with_tx(constrained_gam_tx(None, Some(1)))
        .execute();
}

#[test]
fn test_tx_constraints_allow_slot_window() {
    let mut fixture = fixture_with_gam_target();

    fixture.child_block().with_slot(1).execute();
    fixture
        .child_block()
        .with_slot(2)
        .with_tx(constrained_gam_tx(Some(1), Some(3)))
        .execute();
}

#[test]
fn test_tx_constraints_reject_after_max_slot() {
    let mut fixture = fixture_with_gam_target();

    let err = fixture
        .child_block()
        .with_slot(1)
        .with_tx(constrained_gam_tx(None, Some(0)))
        .execute_err();

    match err.into_base() {
        ExecError::TransactionExpired(max_slot, current_slot) => {
            assert_eq!(max_slot, 0);
            assert_eq!(current_slot, 1);
        }
        err => panic!("Expected TransactionExpired, got: {err:?}"),
    }
}
