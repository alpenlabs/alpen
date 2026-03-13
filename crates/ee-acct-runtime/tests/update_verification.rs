//! Integration tests for update verification and application.
//!
//! These tests verify the complete flow of constructing update operations and
//! ensuring both verified and unconditional application paths yield identical
//! results.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use common::{
    apply_unconditionally, assert_both_paths_succeed, assert_verified_chunks_succeed,
    assert_verified_path_succeeds, build_update_operation, create_deposit_message,
    create_initial_state, simple_chunk,
};
use strata_acct_types::{AccountId, BitcoinAmount, Hash, SubjectId};
use strata_ee_acct_runtime::{EeVerificationInput, UpdateBuilder};
use strata_ee_chain_types::ExecOutputs;
use strata_predicate::PredicateKey;
use strata_simple_ee::SimpleExecutionEnvironment;

#[test]
fn test_empty_update_no_chunks() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let (operation, coinputs, snark_priv) =
        build_update_operation(1, vec![], &[], &initial_state, &snark_state, &ee);

    assert_both_paths_succeed(&initial_state, &operation, &coinputs, &ee);
    assert_verified_path_succeeds(&snark_priv, &[], &ee);
}

#[test]
fn test_single_deposit_no_chunks() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let message = create_deposit_message(dest, value, source, 1);

    let (operation, _coinputs, snark_priv) =
        build_update_operation(1, vec![message], &[], &initial_state, &snark_state, &ee);

    apply_unconditionally(&initial_state, &operation).expect("unconditional path should succeed");
    assert_verified_path_succeeds(&snark_priv, &[], &ee);
}

#[test]
fn test_multiple_deposits_no_chunks() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value1 = BitcoinAmount::from(500u64);
    let value2 = BitcoinAmount::from(750u64);
    let source = AccountId::from([3u8; 32]);

    let message1 = create_deposit_message(dest1, value1, source, 1);
    let message2 = create_deposit_message(dest2, value2, source, 1);

    let (operation, _coinputs, snark_priv) = build_update_operation(
        1,
        vec![message1, message2],
        &[],
        &initial_state,
        &snark_state,
        &ee,
    );

    apply_unconditionally(&initial_state, &operation).expect("unconditional path should succeed");
    assert_verified_path_succeeds(&snark_priv, &[], &ee);
}

#[test]
fn test_empty_update_verified_path() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let (operation, coinputs, snark_priv) =
        build_update_operation(1, vec![], &[], &initial_state, &snark_state, &ee);

    assert_both_paths_succeed(&initial_state, &operation, &coinputs, &ee);
    assert_verified_path_succeeds(&snark_priv, &[], &ee);
}

#[test]
fn test_single_deposit_with_chunk() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create deposit message
    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let message = create_deposit_message(dest, value, source, 1);

    // Build using the builder to get correct pending inputs
    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(&ee, &predicate_key, &[], &[]);
    let mut builder =
        UpdateBuilder::new(1, snark_state, initial_state.clone(), vinput).expect("create builder");

    builder.add_messages(vec![message]).expect("add messages");

    // The deposit message should have created a pending input
    assert_eq!(builder.remaining_input_count(), 1);

    // Create a matching chunk using the builder's state
    let deposit = match &builder.remaining_pending_inputs()[0] {
        strata_ee_acct_types::PendingInputEntry::Deposit(d) => d.clone(),
    };
    let parent = builder.cur_tip_blkid();
    let tip = Hash::new([0xAA; 32]);
    let chunk = simple_chunk(parent, tip, vec![deposit], ExecOutputs::new_empty());

    builder
        .accept_chunk_transition(&chunk)
        .expect("accept chunk should succeed");

    // After accepting, tip should advance and inputs should be consumed
    assert_eq!(builder.cur_tip_blkid(), tip);
    assert_eq!(builder.remaining_input_count(), 0);

    let (operation, coinputs) = builder.build().expect("build should succeed");

    // Unconditional path should succeed
    apply_unconditionally(&initial_state, &operation).expect("unconditional path should succeed");

    // Verified path with chunk proof verification should succeed
    assert_verified_chunks_succeed(&initial_state, &operation, &coinputs, &[chunk], &ee);
}

#[test]
fn test_multiple_deposits_multiple_chunks() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value1 = BitcoinAmount::from(500u64);
    let value2 = BitcoinAmount::from(750u64);
    let source = AccountId::from([3u8; 32]);

    let msg1 = create_deposit_message(dest1, value1, source, 1);
    let msg2 = create_deposit_message(dest2, value2, source, 1);

    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(&ee, &predicate_key, &[], &[]);
    let mut builder =
        UpdateBuilder::new(1, snark_state, initial_state.clone(), vinput).expect("create builder");

    builder
        .add_messages(vec![msg1, msg2])
        .expect("add messages");

    assert_eq!(builder.remaining_input_count(), 2);

    // First chunk: consume first deposit
    let d1 = match &builder.remaining_pending_inputs()[0] {
        strata_ee_acct_types::PendingInputEntry::Deposit(d) => d.clone(),
    };
    let tip1 = Hash::new([0xBB; 32]);
    let chunk1 = simple_chunk(
        builder.cur_tip_blkid(),
        tip1,
        vec![d1],
        ExecOutputs::new_empty(),
    );
    builder
        .accept_chunk_transition(&chunk1)
        .expect("first chunk");

    assert_eq!(builder.cur_tip_blkid(), tip1);
    assert_eq!(builder.remaining_input_count(), 1);

    // Second chunk: consume second deposit
    let d2 = match &builder.remaining_pending_inputs()[0] {
        strata_ee_acct_types::PendingInputEntry::Deposit(d) => d.clone(),
    };
    let tip2 = Hash::new([0xCC; 32]);
    let chunk2 = simple_chunk(
        builder.cur_tip_blkid(),
        tip2,
        vec![d2],
        ExecOutputs::new_empty(),
    );
    builder
        .accept_chunk_transition(&chunk2)
        .expect("second chunk");

    assert_eq!(builder.cur_tip_blkid(), tip2);
    assert_eq!(builder.remaining_input_count(), 0);

    let (operation, coinputs) = builder.build().expect("build");

    apply_unconditionally(&initial_state, &operation).expect("unconditional path should succeed");

    // Verified path with chunk proof verification should succeed
    assert_verified_chunks_succeed(
        &initial_state,
        &operation,
        &coinputs,
        &[chunk1, chunk2],
        &ee,
    );
}
