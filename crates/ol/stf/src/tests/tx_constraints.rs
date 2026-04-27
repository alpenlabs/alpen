//! Tests for transaction slot constraints.

use strata_ol_chain_types_new::{OLTransaction, OLTransactionData, TxConstraints, TxProofs};
use strata_ol_state_types::OLState;

use crate::{
    assembly::{BlockComponents, CompletedBlock},
    context::BlockInfo,
    errors::ExecError,
    test_utils::*,
};

fn setup_empty_genesis_with_gam_target() -> (OLState, CompletedBlock) {
    let mut state = create_test_genesis_state();
    create_empty_account(&mut state, get_test_recipient_account_id());
    let genesis_block = execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![]),
    )
    .expect("genesis should execute");

    (state, genesis_block)
}

fn constrained_gam_tx(min_slot: Option<u64>, max_slot: Option<u64>) -> OLTransaction {
    let target = get_test_recipient_account_id();
    OLTransaction::new(
        OLTransactionData::new_gam(target, vec![])
            .with_constraints(TxConstraints::new(min_slot, max_slot)),
        TxProofs::new_empty(),
    )
}

#[test]
fn test_tx_constraints_allow_min_slot_boundary() {
    let (mut state, genesis_block) = setup_empty_genesis_with_gam_target();

    execute_tx_in_block(
        &mut state,
        genesis_block.header(),
        constrained_gam_tx(Some(1), None),
        1,
        1,
    )
    .expect("transaction should be accepted at min_slot boundary");
}

#[test]
fn test_tx_constraints_reject_before_min_slot() {
    let (mut state, genesis_block) = setup_empty_genesis_with_gam_target();

    let result = execute_tx_in_block(
        &mut state,
        genesis_block.header(),
        constrained_gam_tx(Some(2), None),
        1,
        1,
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::TransactionNotMature(min_slot, current_slot) => {
                assert_eq!(min_slot, 2);
                assert_eq!(current_slot, 1);
            }
            err => panic!("Expected TransactionNotMature, got: {err:?}"),
        },
        Ok(_) => panic!("transaction before min_slot should fail"),
    }
}

#[test]
fn test_tx_constraints_allow_max_slot_boundary() {
    let (mut state, genesis_block) = setup_empty_genesis_with_gam_target();

    execute_tx_in_block(
        &mut state,
        genesis_block.header(),
        constrained_gam_tx(None, Some(1)),
        1,
        1,
    )
    .expect("transaction should be accepted at max_slot boundary");
}

#[test]
fn test_tx_constraints_allow_slot_window() {
    let (mut state, genesis_block) = setup_empty_genesis_with_gam_target();
    let block1 = execute_block(
        &mut state,
        &BlockInfo::new(1_001_000, 1, 1),
        Some(genesis_block.header()),
        BlockComponents::new_empty(),
    )
    .expect("slot 1 parent should execute");

    execute_tx_in_block(
        &mut state,
        block1.header(),
        constrained_gam_tx(Some(1), Some(3)),
        2,
        1,
    )
    .expect("transaction should be accepted inside min/max slot window");
}

#[test]
fn test_tx_constraints_reject_after_max_slot() {
    let (mut state, genesis_block) = setup_empty_genesis_with_gam_target();

    let result = execute_tx_in_block(
        &mut state,
        genesis_block.header(),
        constrained_gam_tx(None, Some(0)),
        1,
        1,
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::TransactionExpired(max_slot, current_slot) => {
                assert_eq!(max_slot, 0);
                assert_eq!(current_slot, 1);
            }
            err => panic!("Expected TransactionExpired, got: {err:?}"),
        },
        Ok(_) => panic!("transaction after max_slot should fail"),
    }
}
