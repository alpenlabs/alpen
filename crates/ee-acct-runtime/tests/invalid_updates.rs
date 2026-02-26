//! Tests for invalid update conditions and error handling.
//!
//! These tests verify that the verification logic correctly rejects
//! malformed or invalid updates.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use common::{
    build_chain_segment_with_deposits, build_update_operation, create_deposit_message,
    create_initial_state, verify_update,
};
use strata_acct_types::{AccountId, BitcoinAmount, SubjectId};
use strata_ee_acct_runtime::ChainSegmentBuilder;
use strata_ee_acct_types::{ExecHeader, PendingInputEntry};
use strata_ee_chain_types::{ExecInputs, SubjectDepositData};
use strata_simple_ee::{SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeaderIntrinsics};
use strata_snark_acct_runtime::ProgramError;

#[test]
#[ignore = "validate_block_inputs is currently disabled in ChainSegmentBuilder"]
fn test_insufficient_pending_inputs() {
    let (_ee_state, exec_state, header) = create_initial_state();
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
    let mut inputs = ExecInputs::new_empty();
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
#[ignore = "validate_block_inputs is currently disabled in ChainSegmentBuilder"]
fn test_wrong_deposit_value_in_block() {
    let (_ee_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit with value 1000
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let deposit = SubjectDepositData::new(dest, value);

    // Create a different deposit with value 500 to put in the block
    let wrong_value = BitcoinAmount::from(500u64);
    let wrong_deposit = SubjectDepositData::new(dest, wrong_value);

    // Build segment with the WRONG deposit value
    let pending_inputs = vec![PendingInputEntry::Deposit(deposit)];
    let mut builder =
        ChainSegmentBuilder::new(ee, exec_state.clone(), header.clone(), pending_inputs);

    let body = SimpleBlockBody::new(vec![]);
    let mut inputs = ExecInputs::new_empty();
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
    let wrong_coinputs = [vec![], vec![]]; // Too many!

    let result = verify_update(&initial_state, &operation, &shared_private, &wrong_coinputs, &ee);

    assert!(matches!(
        result,
        Err(ProgramError::MismatchedCoinputCount {
            expected: 1,
            actual: 2,
        })
    ));

    // Try with too few coinputs — the snark-acct-runtime checks coinput count
    // early, so zero coinputs for one message should also fail.
    let wrong_coinputs2: Vec<Vec<u8>> = vec![]; // Too few!

    let result2 =
        verify_update(&initial_state, &operation, &shared_private, &wrong_coinputs2, &ee);

    assert!(matches!(
        result2,
        Err(ProgramError::MismatchedCoinputCount {
            expected: 1,
            actual: 0,
        })
    ));
}
