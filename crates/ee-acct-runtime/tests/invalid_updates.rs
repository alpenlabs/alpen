//! Tests for invalid update conditions and error handling.
//!
//! These tests verify that the verification logic correctly rejects
//! malformed or invalid updates.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use std::iter;

use common::{
    build_update_operation, create_chunk_transition, create_deposit_message, create_initial_state,
    create_vstate, simple_chunk, verify_update,
};
use strata_acct_types::{AccountId, BitcoinAmount, Hash, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{BuilderError, EeVerificationInput, UpdateBuilder};
use strata_ee_acct_types::{PendingInputEntry, UpdateExtraData};
use strata_ee_chain_types::{ExecInputs, ExecOutputs, SequenceTracker, SubjectDepositData};
use strata_predicate::PredicateKey;
use strata_simple_ee::SimpleExecutionEnvironment;
use strata_snark_acct_runtime::ProgramError;
use strata_snark_acct_types::{
    LedgerRefs, OutputTransfer, ProofState, UpdateOperationData, UpdateOutputs,
};

// ---- Coinput validation ----

#[test]
fn test_mismatched_coinput_count() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let message = create_deposit_message(dest, value, source, 1);

    let (operation, _coinputs, _snark_priv) =
        build_update_operation(1, vec![message], &[], &initial_state, &snark_state, &ee);

    // Too many coinputs
    let wrong_coinputs = [vec![], vec![]];
    let result = verify_update(&initial_state, &operation, &wrong_coinputs, &ee);
    assert!(matches!(
        result,
        Err(ProgramError::MismatchedCoinputCount {
            expected: 1,
            actual: 2,
        })
    ));

    // Too few coinputs
    let wrong_coinputs2: Vec<Vec<u8>> = vec![];
    let result2 = verify_update(&initial_state, &operation, &wrong_coinputs2, &ee);
    assert!(matches!(
        result2,
        Err(ProgramError::MismatchedCoinputCount {
            expected: 1,
            actual: 0,
        })
    ));
}

#[test]
fn test_nonempty_coinput_rejected() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let message = create_deposit_message(dest, value, source, 1);

    let (operation, _coinputs, _snark_priv) =
        build_update_operation(1, vec![message], &[], &initial_state, &snark_state, &ee);

    // Non-empty coinput should be rejected (EE requires empty coinputs)
    let nonempty_coinputs = vec![vec![1, 2, 3]];
    let result = verify_update(&initial_state, &operation, &nonempty_coinputs, &ee);
    assert!(
        matches!(
            result,
            Err(ProgramError::AtMessage { idx: 0, ref inner })
            if matches!(**inner, ProgramError::MalformedCoinput)
        ),
        "expected MalformedCoinput at message 0, got: {result:?}"
    );
}

// ---- Output / extra data validation (constructed manually to bypass builder validation) ----

#[test]
fn test_output_mismatch_fails() {
    let (initial_state, _snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Manually construct operation data with outputs that don't match
    // accumulated chunks (there are none, so expected should be empty).
    let mut wrong_outputs = UpdateOutputs::new_empty();
    wrong_outputs
        .try_extend_transfers(iter::once(OutputTransfer::new(
            AccountId::from([99u8; 32]),
            BitcoinAmount::from(500u64),
        )))
        .unwrap();

    let extra_data = UpdateExtraData::new(initial_state.last_exec_blkid(), 0, 0);
    let extra_data_buf = encode_to_vec(&extra_data).unwrap();

    let operation = UpdateOperationData::new(
        1,
        ProofState::new(Hash::default(), 0),
        vec![],
        LedgerRefs::new_empty(),
        wrong_outputs,
        extra_data_buf,
    );

    let coinputs: Vec<Vec<u8>> = vec![];
    let result = verify_update(&initial_state, &operation, &coinputs, &ee);

    assert!(
        matches!(result, Err(ProgramError::UnsatisfiedObligations)),
        "expected UnsatisfiedObligations, got: {result:?}"
    );
}

#[test]
fn test_extra_data_tip_mismatch() {
    let (initial_state, _snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Manually construct operation data with a wrong tip.
    let wrong_tip = Hash::new([0xAA; 32]);
    let extra_data = UpdateExtraData::new(wrong_tip, 0, 0);
    let extra_data_buf = encode_to_vec(&extra_data).unwrap();

    let operation = UpdateOperationData::new(
        1,
        ProofState::new(Hash::default(), 0),
        vec![],
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        extra_data_buf,
    );

    let coinputs: Vec<Vec<u8>> = vec![];
    let result = verify_update(&initial_state, &operation, &coinputs, &ee);

    assert!(
        matches!(result, Err(ProgramError::InvalidExtraData)),
        "expected InvalidExtraData, got: {result:?}"
    );
}

// ---- Builder chunk validation ----

#[test]
fn test_builder_rejects_wrong_parent() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(&ee, &predicate_key, &[], &[]);
    let mut builder =
        UpdateBuilder::new(1, snark_state, initial_state, vinput).expect("create builder");

    // Chunk with wrong parent
    let wrong_parent = Hash::new([0xCC; 32]);
    let tip = Hash::new([0xDD; 32]);
    let chunk = simple_chunk(wrong_parent, tip, vec![], ExecOutputs::new_empty());

    let result = builder.accept_chunk_transition(&chunk);
    assert!(
        matches!(result, Err(BuilderError::ChainLinkage { .. })),
        "expected ChainLinkage, got: {result:?}"
    );
}

#[test]
fn test_builder_rejects_wrong_deposit() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let source = AccountId::from([2u8; 32]);
    let message = create_deposit_message(dest, value, source, 1);

    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(&ee, &predicate_key, &[], &[]);
    let mut builder =
        UpdateBuilder::new(1, snark_state, initial_state, vinput).expect("create builder");

    builder.add_messages(vec![message]).expect("add messages");

    assert_eq!(builder.remaining_input_count(), 1);

    // Chunk with a different deposit than what's pending
    let wrong_dest = SubjectId::from([99u8; 32]);
    let wrong_deposit = SubjectDepositData::new(wrong_dest, value);
    let chunk = simple_chunk(
        builder.cur_tip_blkid(),
        Hash::new([0xEE; 32]),
        vec![wrong_deposit],
        ExecOutputs::new_empty(),
    );

    let result = builder.accept_chunk_transition(&chunk);
    assert!(
        matches!(result, Err(BuilderError::InputMismatch { .. })),
        "expected InputMismatch, got: {result:?}"
    );
}

#[test]
fn test_builder_advances_tip() {
    let (initial_state, snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let dest1 = SubjectId::from([1u8; 32]);
    let dest2 = SubjectId::from([2u8; 32]);
    let value = BitcoinAmount::from(500u64);
    let source = AccountId::from([3u8; 32]);

    let msg1 = create_deposit_message(dest1, value, source, 1);
    let msg2 = create_deposit_message(dest2, value, source, 1);

    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(&ee, &predicate_key, &[], &[]);
    let mut builder =
        UpdateBuilder::new(1, snark_state, initial_state, vinput).expect("create builder");

    builder
        .add_messages(vec![msg1, msg2])
        .expect("add messages");

    let initial_tip = builder.cur_tip_blkid();

    // First chunk
    let d1 = match &builder.remaining_pending_inputs()[0] {
        PendingInputEntry::Deposit(d) => d.clone(),
    };
    let tip1 = Hash::new([0xAA; 32]);
    let chunk1 = simple_chunk(initial_tip, tip1, vec![d1], ExecOutputs::new_empty());
    builder.accept_chunk_transition(&chunk1).expect("chunk 1");

    assert_eq!(builder.cur_tip_blkid(), tip1);
    assert_ne!(builder.cur_tip_blkid(), initial_tip);
    assert_eq!(builder.remaining_input_count(), 1);

    // Second chunk — parent must be tip1
    let d2 = match &builder.remaining_pending_inputs()[0] {
        PendingInputEntry::Deposit(d) => d.clone(),
    };
    let tip2 = Hash::new([0xBB; 32]);
    let chunk2 = simple_chunk(tip1, tip2, vec![d2], ExecOutputs::new_empty());
    builder.accept_chunk_transition(&chunk2).expect("chunk 2");

    assert_eq!(builder.cur_tip_blkid(), tip2);
    assert_eq!(builder.remaining_input_count(), 0);
}

// ---- process_decoded_transition tests (verification-side) ----

#[test]
fn test_process_decoded_transition_happy_path() {
    let (initial_state, _snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let parent = initial_state.last_exec_blkid();
    let tip = Hash::new([0xBB; 32]);

    let transition = create_chunk_transition(
        parent,
        tip,
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );

    let predicate_key = PredicateKey::always_accept();
    let mut vstate = create_vstate(
        &ee,
        &predicate_key,
        &initial_state,
        UpdateOutputs::new_empty(),
    );

    let pending: Vec<PendingInputEntry> = vec![];
    let mut tracker = SequenceTracker::new(&pending);

    let result = vstate.process_decoded_transition(&transition, &mut tracker);
    assert!(result.is_ok(), "happy path should succeed: {result:?}");
    assert_eq!(vstate.cur_verified_exec_blkid(), tip);
}

#[test]
fn test_chain_linkage_mismatch() {
    let (initial_state, _snark_state) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    let wrong_parent = Hash::new([0xCC; 32]);
    let tip = Hash::new([0xDD; 32]);

    let transition = create_chunk_transition(
        wrong_parent,
        tip,
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );

    let predicate_key = PredicateKey::always_accept();
    let mut vstate = create_vstate(
        &ee,
        &predicate_key,
        &initial_state,
        UpdateOutputs::new_empty(),
    );

    let pending: Vec<PendingInputEntry> = vec![];
    let mut tracker = SequenceTracker::new(&pending);

    let result = vstate.process_decoded_transition(&transition, &mut tracker);
    assert!(
        matches!(
            result,
            Err(strata_ee_acct_types::EnvError::MismatchedChainSegment)
        ),
        "expected MismatchedChainSegment, got: {result:?}"
    );
}

#[test]
fn test_deposit_mismatch_in_chunk() {
    let ee = SimpleExecutionEnvironment;

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let deposit = SubjectDepositData::new(dest, value);

    let initial_state = strata_ee_acct_types::EeAccountState::new(
        Hash::new([0u8; 32]),
        BitcoinAmount::from(0u64),
        vec![PendingInputEntry::Deposit(deposit)],
        Vec::new(),
    );

    let parent = initial_state.last_exec_blkid();
    let tip = Hash::new([0xEE; 32]);

    let wrong_dest = SubjectId::from([2u8; 32]);
    let wrong_deposit = SubjectDepositData::new(wrong_dest, value);
    let mut inputs = ExecInputs::new_empty();
    inputs.add_subject_deposit(wrong_deposit);

    let transition = create_chunk_transition(parent, tip, inputs, ExecOutputs::new_empty());

    let predicate_key = PredicateKey::always_accept();
    let mut vstate = create_vstate(
        &ee,
        &predicate_key,
        &initial_state,
        UpdateOutputs::new_empty(),
    );

    let pending = initial_state.pending_inputs().to_vec();
    let mut tracker = SequenceTracker::new(&pending);

    let result = vstate.process_decoded_transition(&transition, &mut tracker);
    assert!(
        matches!(
            result,
            Err(strata_ee_acct_types::EnvError::InconsistentChunkIo)
        ),
        "expected InconsistentChunkIo, got: {result:?}"
    );
}

#[test]
fn test_input_count_mismatch() {
    let ee = SimpleExecutionEnvironment;

    let initial_state = strata_ee_acct_types::EeAccountState::new(
        Hash::new([0u8; 32]),
        BitcoinAmount::from(0u64),
        Vec::new(),
        Vec::new(),
    );

    let parent = initial_state.last_exec_blkid();
    let tip = Hash::new([0xFF; 32]);

    let dest = SubjectId::from([1u8; 32]);
    let value = BitcoinAmount::from(1000u64);
    let deposit = SubjectDepositData::new(dest, value);
    let mut inputs = ExecInputs::new_empty();
    inputs.add_subject_deposit(deposit);

    let transition = create_chunk_transition(parent, tip, inputs, ExecOutputs::new_empty());

    let predicate_key = PredicateKey::always_accept();
    let mut vstate = create_vstate(
        &ee,
        &predicate_key,
        &initial_state,
        UpdateOutputs::new_empty(),
    );

    let pending: Vec<PendingInputEntry> = vec![];
    let mut tracker = SequenceTracker::new(&pending);

    let result = vstate.process_decoded_transition(&transition, &mut tracker);
    assert!(
        matches!(
            result,
            Err(strata_ee_acct_types::EnvError::InconsistentChunkIo)
        ),
        "expected InconsistentChunkIo, got: {result:?}"
    );
}
