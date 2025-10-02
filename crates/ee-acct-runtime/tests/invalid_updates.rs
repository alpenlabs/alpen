//! Tests for invalid update conditions and error handling.
//!
//! These tests verify that the verification logic correctly rejects
//! malformed or invalid updates.

#![allow(unused_crate_dependencies)]

mod common;

use common::{
    build_chain_segment_with_deposits, build_update_operation, create_deposit_message,
    create_initial_state,
};
use strata_acct_types::{AccountId, BitcoinAmount, SubjectId};
use strata_ee_acct_runtime::ChainSegmentBuilder;
use strata_ee_acct_types::{EnvError, ExecHeader, PendingInputEntry};
use strata_ee_chain_types::{BlockInputs, SubjectDepositData};
use strata_simple_ee::{SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeaderIntrinsics};

#[test]
fn test_mismatched_processed_inputs_count() {
    let (mut initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit and add it to initial state
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);
    let message = create_deposit_message(dest, value, source, 1);

    // Add to tracked balance and pending inputs
    initial_state.add_tracked_balance(value);
    initial_state.add_pending_input(PendingInputEntry::Deposit(deposit.clone()));

    // Build segment that consumes the deposit
    let segment =
        build_chain_segment_with_deposits(ee, exec_state.clone(), header.clone(), vec![deposit]);

    // Build update operation
    let (mut operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Tamper with the processed_inputs count in extra_data
    // This should cause ConflictingPublicState error
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_ee_acct_types::UpdateExtraData;

    let mut extra: UpdateExtraData = decode_buf_exact(operation.extra_data()).unwrap();
    let tampered_extra = UpdateExtraData::new(
        *extra.new_tip_blkid(),
        extra.processed_inputs() + 1, // Wrong count!
        *extra.processed_fincls(),
    );
    let tampered_extra_buf = encode_to_vec(&tampered_extra).unwrap();

    // Replace extra_data (we need to rebuild the operation)
    let tampered_operation = strata_snark_acct_types::UpdateOperationData::new(
        operation.seq_no(),
        operation.new_state().clone(),
        operation.processed_messages().to_vec(),
        operation.ledger_refs().clone(),
        operation.outputs().clone(),
        tampered_extra_buf,
    );

    // Try to verify - should fail
    let mut test_state = initial_state.clone();
    let result = strata_ee_acct_runtime::verify_and_apply_update_operation(
        &mut test_state,
        &tampered_operation,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    );

    assert!(matches!(result, Err(EnvError::ConflictingPublicState)));
}

#[test]
fn test_mismatched_segment_count() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);
    let message = create_deposit_message(dest, value, source, 1);

    // Build segment
    let segment =
        build_chain_segment_with_deposits(ee, exec_state.clone(), header.clone(), vec![deposit]);

    // Build update operation with the segment
    let (operation, mut shared_private, coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Tamper with shared_private to remove the segment
    use strata_codec::encode_to_vec;
    let prev_header_buf = encode_to_vec(&header).unwrap();
    let prev_state_buf = encode_to_vec(&exec_state).unwrap();

    shared_private = strata_ee_acct_runtime::SharedPrivateInput::new(
        vec![], // Empty segments!
        prev_header_buf,
        prev_state_buf,
    );

    // Try to verify - should fail with MismatchedChainSegment
    let mut test_state = initial_state.clone();
    let result = strata_ee_acct_runtime::verify_and_apply_update_operation(
        &mut test_state,
        &operation,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    );

    assert!(matches!(result, Err(EnvError::MismatchedChainSegment)));
}

#[test]
fn test_insufficient_pending_inputs() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let deposit = SubjectDepositData::new(dest, value);

    // Build a segment that tries to consume a deposit, but don't add it to pending inputs
    let pending_inputs = vec![]; // Empty!
    let mut builder =
        ChainSegmentBuilder::new(ee, exec_state.clone(), header.clone(), pending_inputs);

    let body = SimpleBlockBody::new(vec![]);
    let mut inputs = BlockInputs::new_empty();
    inputs.add_subject_deposit(deposit);

    let intrinsics = SimpleHeaderIntrinsics {
        parent_blkid: header.compute_block_id(),
        index: header.index() + 1,
    };

    // This should fail because there are no pending inputs
    let result = builder.append_block_body(&intrinsics, body, inputs);

    assert!(result.is_err());
}

#[test]
fn test_wrong_deposit_value_in_block() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit with value 1000
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);
    let message = create_deposit_message(dest, value, source, 1);

    // Create a different deposit with value 500 to put in the block
    let wrong_value = BitcoinAmount::from(500u64);
    let wrong_deposit = SubjectDepositData::new(dest, wrong_value);

    // Build segment with the WRONG deposit value
    let pending_inputs = vec![PendingInputEntry::Deposit(deposit)];
    let mut builder =
        ChainSegmentBuilder::new(ee, exec_state.clone(), header.clone(), pending_inputs);

    let body = SimpleBlockBody::new(vec![]);
    let mut inputs = BlockInputs::new_empty();
    inputs.add_subject_deposit(wrong_deposit); // Wrong value!

    let intrinsics = SimpleHeaderIntrinsics {
        parent_blkid: header.compute_block_id(),
        index: header.index() + 1,
    };

    // This should fail because the deposit value doesn't match
    let result = builder.append_block_body(&intrinsics, body, inputs);

    assert!(result.is_err());
}

#[test]
fn test_mismatched_coinput_count() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);
    let message = create_deposit_message(dest, value, source, 1);

    // Build segment
    let segment =
        build_chain_segment_with_deposits(ee, exec_state.clone(), header.clone(), vec![deposit]);

    // Build update operation
    let (operation, shared_private, _coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Try to verify with wrong number of coinputs (too many)
    let mut test_state = initial_state.clone();
    let wrong_coinputs = vec![vec![], vec![]]; // Too many!

    let result = strata_ee_acct_runtime::verify_and_apply_update_operation(
        &mut test_state,
        &operation,
        wrong_coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    );

    assert!(matches!(result, Err(EnvError::MismatchedCoinputCnt)));

    // Try with too few
    let mut test_state2 = initial_state.clone();
    let wrong_coinputs2: Vec<Vec<u8>> = vec![]; // Too few!

    let result2 = strata_ee_acct_runtime::verify_and_apply_update_operation(
        &mut test_state2,
        &operation,
        wrong_coinputs2.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    );

    assert!(matches!(result2, Err(EnvError::MismatchedCoinputCnt)));
}
