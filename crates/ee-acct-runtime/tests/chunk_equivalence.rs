//! Chunk-based equivalence tests.
//!
//! These tests verify that chunk-based processing is equivalent to
//! unconditional update application, which is the key correctness property
//! for our chunk-based proof system.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use common::{
    build_chain_segment_with_deposits, build_chunk_operation, build_update_operation,
    create_deposit_message, create_initial_state, update_to_single_chunk_op,
};
use strata_acct_types::{AccountId, BitcoinAmount, SubjectId};
use strata_ee_chain_types::SubjectDepositData;
use strata_simple_ee::SimpleExecutionEnvironment;

/// Test that a single chunk covering the entire update is equivalent to
/// unconditional application.
///
/// This is the most basic equivalence property: when all blocks are in one
/// chunk, it should produce the same result as applying the update unconditionally.
#[test]
fn test_single_chunk_equivalence_empty_update() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create an update with no messages and no segments
    let (operation, shared_private, coinputs) =
        build_update_operation(1, vec![], vec![], &initial_state, &header, &exec_state);

    // Convert to single chunk
    let chunk_op = update_to_single_chunk_op(&operation, &initial_state);

    // Apply chunk with verification
    let mut state_chunk = initial_state.clone();
    strata_ee_acct_runtime::verify_and_apply_chunk_operation(
        &mut state_chunk,
        &chunk_op,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("chunk verification should succeed");

    // Apply unconditionally
    let mut state_unconditional = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = operation.into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut state_unconditional,
        &input_data,
    )
    .expect("unconditional application should succeed");

    // States should match
    assert_eq!(
        state_chunk, state_unconditional,
        "Single chunk and unconditional application should yield identical states"
    );
}

#[test]
fn test_single_chunk_equivalence_with_deposit() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create a deposit
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let deposit = SubjectDepositData::new(dest, value);
    let message = create_deposit_message(dest, value, source, 1);

    // Build chain segment
    let segment = build_chain_segment_with_deposits(
        ee,
        exec_state.clone(),
        header.clone(),
        vec![deposit.clone()],
    );

    // Build update operation
    let (operation, shared_private, coinputs) = build_update_operation(
        1,
        vec![message],
        vec![segment],
        &initial_state,
        &header,
        &exec_state,
    );

    // Convert to single chunk
    let chunk_op = update_to_single_chunk_op(&operation, &initial_state);

    // Apply chunk with verification
    let mut state_chunk = initial_state.clone();
    strata_ee_acct_runtime::verify_and_apply_chunk_operation(
        &mut state_chunk,
        &chunk_op,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("chunk verification should succeed");

    // Apply unconditionally
    let mut state_unconditional = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = operation.into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut state_unconditional,
        &input_data,
    )
    .expect("unconditional application should succeed");

    // States should match
    assert_eq!(
        state_chunk, state_unconditional,
        "Single chunk and unconditional application should yield identical states"
    );
}

#[test]
fn test_single_chunk_equivalence_multiple_deposits() {
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

    let message1 = create_deposit_message(dest1, value1, source, 1);
    let message2 = create_deposit_message(dest2, value2, source, 1);

    // Build chain segment with two deposits
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

    // Convert to single chunk
    let chunk_op = update_to_single_chunk_op(&operation, &initial_state);

    // Apply chunk with verification
    let mut state_chunk = initial_state.clone();
    strata_ee_acct_runtime::verify_and_apply_chunk_operation(
        &mut state_chunk,
        &chunk_op,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("chunk verification should succeed");

    // Apply unconditionally
    let mut state_unconditional = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = operation.into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut state_unconditional,
        &input_data,
    )
    .expect("unconditional application should succeed");

    // States should match
    assert_eq!(
        state_chunk, state_unconditional,
        "Single chunk and unconditional application should yield identical states"
    );
}

/// Test multi-chunk equivalence with multiple segments.
///
/// This verifies the key property: applying chunks sequentially should produce
/// the same result as unconditional application of the full update.
#[test]
fn test_multi_chunk_equivalence() {
    let (initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    use strata_ee_acct_runtime::ChainSegmentBuilder;
    use strata_ee_acct_types::{ExecHeader, PendingInputEntry};
    use strata_ee_chain_types::BlockInputs;
    use strata_simple_ee::{SimpleBlockBody, SimpleHeaderIntrinsics};

    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value1 = BitcoinAmount::from(500u64);
    let value2 = BitcoinAmount::from(750u64);
    let source = AccountId::from([3u8; 32]);

    let deposit1 = SubjectDepositData::new(dest1, value1);
    let deposit2 = SubjectDepositData::new(dest2, value2);

    let message1 = create_deposit_message(dest1, value1, source, 1);
    let message2 = create_deposit_message(dest2, value2, source, 1);

    // Build segment1 manually so we can track the state
    let mut builder1 = ChainSegmentBuilder::new(
        ee,
        exec_state.clone(),
        header.clone(),
        vec![PendingInputEntry::Deposit(deposit1.clone())],
    );
    let body1 = SimpleBlockBody::new(vec![]);
    let mut inputs1 = BlockInputs::new_empty();
    inputs1.add_subject_deposit(deposit1);
    let intrinsics1 = SimpleHeaderIntrinsics {
        parent_blkid: header.compute_block_id(),
        index: header.index() + 1,
    };
    builder1
        .append_block_body(&intrinsics1, body1, inputs1)
        .expect("segment1 block should succeed");

    // Get state/header after segment1
    let header_after_seg1 = builder1.current_header().clone();
    let state_after_seg1 = builder1.current_state().clone();
    let segment1 = builder1.build();

    // Build chunk1
    let (chunk1_op, private1, coinputs1) = build_chunk_operation(
        1,
        vec![message1.clone()],
        vec![segment1.clone()],
        &initial_state,
        &header,
        &exec_state,
    );

    // Apply chunk1 to get intermediate EE account state
    let mut state_after_chunk1 = initial_state.clone();
    strata_ee_acct_runtime::verify_and_apply_chunk_operation(
        &mut state_after_chunk1,
        &chunk1_op,
        coinputs1.iter().map(|v| v.as_slice()),
        &private1,
        &ee,
    )
    .expect("chunk1 should succeed");

    // Build segment2 starting from exec state after segment1
    let mut builder2 = ChainSegmentBuilder::new(
        ee,
        state_after_seg1.clone(),
        header_after_seg1.clone(),
        vec![PendingInputEntry::Deposit(deposit2.clone())],
    );
    let body2 = SimpleBlockBody::new(vec![]);
    let mut inputs2 = BlockInputs::new_empty();
    inputs2.add_subject_deposit(deposit2);
    let intrinsics2 = SimpleHeaderIntrinsics {
        parent_blkid: header_after_seg1.compute_block_id(),
        index: header_after_seg1.index() + 1,
    };
    builder2
        .append_block_body(&intrinsics2, body2, inputs2)
        .expect("segment2 block should succeed");
    let segment2 = builder2.build();

    // Build chunk2
    let (chunk2_op, private2, coinputs2) = build_chunk_operation(
        2,
        vec![message2.clone()],
        vec![segment2.clone()],
        &state_after_chunk1,
        &header_after_seg1,
        &state_after_seg1,
    );

    // Apply chunk2
    let mut state_chunks = state_after_chunk1.clone();
    strata_ee_acct_runtime::verify_and_apply_chunk_operation(
        &mut state_chunks,
        &chunk2_op,
        coinputs2.iter().map(|v| v.as_slice()),
        &private2,
        &ee,
    )
    .expect("chunk2 should succeed");

    // Apply full update unconditionally (both segments combined)
    let (full_operation, _full_private, _full_coinputs) = build_update_operation(
        1,
        vec![message1, message2],
        vec![segment1, segment2],
        &initial_state,
        &header,
        &exec_state,
    );

    let mut state_unconditional = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = full_operation.into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut state_unconditional,
        &input_data,
    )
    .expect("unconditional application should succeed");

    // States should match
    assert_eq!(
        state_chunks, state_unconditional,
        "Multi-chunk and unconditional application should yield identical states"
    );
}
