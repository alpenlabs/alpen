//! Integration tests for update verification and application.
//!
//! These tests verify the complete flow of constructing chain segments,
//! building update operations, and ensuring both verified and unconditional
//! application paths yield identical results.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use std::collections::BTreeMap;

use common::{
    assert_update_paths_match, build_chain_segment_with_deposits, build_update_operation,
    create_deposit_message, create_initial_state,
};
use strata_acct_types::{AccountId, BitcoinAmount, SubjectId};
use strata_ee_acct_runtime::ChainSegmentBuilder;
use strata_ee_acct_types::{ExecHeader, ExecPartialState, PendingInputEntry};
use strata_ee_chain_types::{BlockInputs, SubjectDepositData};
use strata_simple_ee::{
    SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeader, SimpleHeaderIntrinsics,
    SimplePartialState, SimpleTransaction,
};

#[test]
fn test_empty_update_no_segments() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create an update with no messages and no segments
    let (operation, shared_private, coinputs) =
        build_update_operation(1, vec![], vec![], &initial_state, &header, &exec_state);

    // Both paths should produce the same result
    assert_update_paths_match(&initial_state, &operation, &shared_private, &coinputs, &ee);
}

#[test]
fn test_single_deposit_single_block() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);

    // Create message entry for the deposit
    let message = create_deposit_message(dest, value, source, 1);

    // Build chain segment that processes the deposit
    let segment = build_chain_segment_with_deposits(
        ee,
        exec_state.clone(),
        header.clone(),
        vec![deposit.clone()],
    );

    // Build the update operation using UpdateBuilder
    let (operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Both application paths should yield the same final state
    assert_update_paths_match(&initial_state, &operation, &shared_private, &coinputs, &ee);
}

#[test]
fn test_multiple_deposits_single_segment() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create multiple deposits
    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value1 = BitcoinAmount::from(500u64);
    let value2 = BitcoinAmount::from(750u64);
    let source = AccountId::from([3u8; 32]);

    let deposit1 = SubjectDepositData::new(dest1, value1);
    let deposit2 = SubjectDepositData::new(dest2, value2);

    // Create message entries
    let message1 = create_deposit_message(dest1, value1, source, 1);
    let message2 = create_deposit_message(dest2, value2, source, 1);

    // Build chain segment
    let segment = build_chain_segment_with_deposits(
        ee,
        exec_state.clone(),
        header.clone(),
        vec![deposit1, deposit2],
    );

    // Build update operation
    let (operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message1, message2],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Verify both paths match
    assert_update_paths_match(&initial_state, &operation, &shared_private, &coinputs, &ee);
}

#[test]
fn test_multiple_blocks_in_segment() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create deposits
    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value1 = BitcoinAmount::from(500u64);
    let value2 = BitcoinAmount::from(750u64);
    let source = AccountId::from([3u8; 32]);

    let deposit1 = SubjectDepositData::new(dest1, value1);
    let deposit2 = SubjectDepositData::new(dest2, value2);

    // Create messages
    let message1 = create_deposit_message(dest1, value1, source, 1);
    let message2 = create_deposit_message(dest2, value2, source, 1);

    // Build a segment with two blocks, each consuming one deposit
    let pending_inputs = vec![
        PendingInputEntry::Deposit(deposit1.clone()),
        PendingInputEntry::Deposit(deposit2.clone()),
    ];

    let mut builder =
        ChainSegmentBuilder::new(ee, exec_state.clone(), header.clone(), pending_inputs);

    // First block
    let body1 = SimpleBlockBody::new(vec![]);
    let mut inputs1 = BlockInputs::new_empty();
    inputs1.add_subject_deposit(deposit1);
    let intrinsics1 = SimpleHeaderIntrinsics {
        parent_blkid: header.compute_block_id(),
        index: header.index() + 1,
    };
    builder
        .append_block_body(&intrinsics1, body1, inputs1)
        .expect("first block should succeed");

    // Second block
    let body2 = SimpleBlockBody::new(vec![]);
    let mut inputs2 = BlockInputs::new_empty();
    inputs2.add_subject_deposit(deposit2);
    let intrinsics2 = SimpleHeaderIntrinsics {
        parent_blkid: builder.current_header().compute_block_id(),
        index: builder.current_header().index() + 1,
    };
    builder
        .append_block_body(&intrinsics2, body2, inputs2)
        .expect("second block should succeed");

    let segment = builder.build();

    // Build update operation
    let (operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message1, message2],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Verify both paths match
    assert_update_paths_match(&initial_state, &operation, &shared_private, &coinputs, &ee);
}

#[test]
fn test_deposits_with_transactions() {
    let (initial_state, _exec_state, mut header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Set up initial execution state with an existing account
    let alice = SubjectId::from([10u8; 32]);
    let bob = SubjectId::from([20u8; 32]);
    let mut initial_accounts = BTreeMap::new();
    initial_accounts.insert(alice, 1000u64);
    let exec_state = SimplePartialState::new(initial_accounts.clone());

    // Create a matching header for this exec_state
    let state_root = exec_state.compute_state_root().expect("compute state root");
    header = SimpleHeader::new(header.compute_block_id(), state_root, header.index());

    // Create a deposit for bob
    let deposit_value = BitcoinAmount::from(500u64);
    let source = AccountId::from([3u8; 32]);
    let deposit = SubjectDepositData::new(bob, deposit_value);
    let message = create_deposit_message(bob, deposit_value, source, 1);

    // Build segment with deposit and transactions including outputs
    let pending_inputs = vec![PendingInputEntry::Deposit(deposit.clone())];
    let mut builder =
        ChainSegmentBuilder::new(ee, exec_state.clone(), header.clone(), pending_inputs);

    // Create a block with the deposit, internal transfer, and output transactions
    let dest_account = AccountId::from([99u8; 32]);
    let charlie = SubjectId::from([30u8; 32]);

    let transfer = SimpleTransaction::Transfer {
        from: alice,
        to: bob,
        value: 100,
    };
    let emit_transfer = SimpleTransaction::EmitTransfer {
        from: alice,
        dest: dest_account,
        value: 200,
    };
    let emit_message = SimpleTransaction::EmitMessage {
        from: alice,
        dest_account,
        dest_subject: charlie,
        value: 150,
        data: vec![1, 2, 3, 4],
    };

    let body = SimpleBlockBody::new(vec![transfer, emit_transfer, emit_message]);
    let mut inputs = BlockInputs::new_empty();
    inputs.add_subject_deposit(deposit);

    let intrinsics = SimpleHeaderIntrinsics {
        parent_blkid: header.compute_block_id(),
        index: header.index() + 1,
    };

    builder
        .append_block_body(&intrinsics, body, inputs)
        .expect("block should succeed");

    let segment = builder.build();

    // Build update operation
    let (operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Verify both paths match
    assert_update_paths_match(&initial_state, &operation, &shared_private, &coinputs, &ee);
}
