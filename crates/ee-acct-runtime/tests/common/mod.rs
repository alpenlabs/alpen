//! Common test utilities for integration tests.

#![allow(unreachable_pub, reason = "test utilities")]
#![allow(dead_code, reason = "utilities used by different test files")]

use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ChunkInput, EePrivateInput, EeSnarkAccountProgram, EeVerificationInput, EeVerificationState,
    UpdateBuilder,
};
use strata_ee_acct_types::{EeAccountState, EnvError};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs, SubjectDepositData};
use strata_msg_fmt::Msg as MsgTrait;
use strata_simple_ee::SimpleExecutionEnvironment;
use strata_snark_acct_runtime::{
    Coinput, IInnerState, PrivateInput as SnarkPrivateInput, ProgramResult,
};
use strata_snark_acct_types::{
    MessageEntry, ProofState, SnarkAccountState, UpdateManifest, UpdateOperationData,
    UpdateOutputs, UpdateProofPubParams,
};

/// Creates a [`SnarkAccountState`] that matches the given [`EeAccountState`].
///
/// Uses default hash for inner_state (matches `EeAccountState::compute_state_root()`
/// which currently returns `Hash::default()`).
pub fn make_snark_state(ee_state: &EeAccountState) -> SnarkAccountState {
    SnarkAccountState {
        update_vk: Vec::new().into(),
        proof_state: ProofState::new(ee_state.compute_state_root(), 0),
        seq_no: 0,
        inbox_mmr: strata_acct_types::Mmr64 {
            entries: 0,
            roots: Default::default(),
        },
    }
}

/// Applies an update unconditionally (DA reconstruction path).
pub fn apply_unconditionally(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
) -> ProgramResult<(), EnvError> {
    let mut state = initial_state.clone();

    let manifest = UpdateManifest::new(
        ProofState::new(Hash::default(), operation.processed_messages().len() as u64),
        operation.extra_data().to_vec(),
        operation.processed_messages().to_vec(),
    );

    strata_ee_acct_runtime::process_update_unconditionally::<SimpleExecutionEnvironment>(
        &mut state, &manifest,
    )?;

    Ok(())
}

/// Runs the verified (SNARK proof) path and returns the result.
pub fn verify_update(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    coinputs: &[Vec<u8>],
    ee: &SimpleExecutionEnvironment,
) -> ProgramResult<(), EnvError> {
    let pub_params = UpdateProofPubParams::new(
        ProofState::new(Hash::default(), 0),
        ProofState::new(Hash::default(), operation.processed_messages().len() as u64),
        operation.processed_messages().to_vec(),
        operation.ledger_refs().clone(),
        operation.outputs().clone(),
        operation.extra_data().to_vec(),
    );

    let coinputs_typed: Vec<Coinput> = coinputs.iter().map(|v| Coinput::new(v.clone())).collect();

    let snark_priv =
        SnarkPrivateInput::new(pub_params, initial_state.as_ssz_bytes(), coinputs_typed);

    let vinput = EeVerificationInput::new(ee, &[], &[]);

    let program = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
    strata_snark_acct_runtime::verify_and_process_update(&program, &snark_priv, vinput)
}

/// Asserts that both verified and unconditional paths succeed.
pub fn assert_both_paths_succeed(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    coinputs: &[Vec<u8>],
    ee: &SimpleExecutionEnvironment,
) {
    verify_update(initial_state, operation, coinputs, ee).expect("verified path should succeed");

    apply_unconditionally(initial_state, operation).expect("unconditional path should succeed");
}

/// Creates a simple initial state for testing.
pub(crate) fn create_initial_state() -> (EeAccountState, SnarkAccountState) {
    let ee_state = EeAccountState::new(
        vec![0x01],
        Hash::new([0u8; 32]),
        BitcoinAmount::from(0u64),
        Vec::new(),
        Vec::new(),
    );

    let snark_state = make_snark_state(&ee_state);

    (ee_state, snark_state)
}

/// Helper to create a deposit message entry.
pub(crate) fn create_deposit_message(
    dest: SubjectId,
    value: BitcoinAmount,
    source: AccountId,
    incl_epoch: u32,
) -> MessageEntry {
    use strata_ee_acct_types::{DEPOSIT_MSG_TYPE, DepositMsgData};
    use strata_msg_fmt::OwnedMsg;

    let deposit_data = DepositMsgData::new(dest);
    let body = encode_to_vec(&deposit_data).expect("encode deposit data");

    let msg = OwnedMsg::new(DEPOSIT_MSG_TYPE, body).expect("create message");
    let payload_data = msg.to_vec();

    let payload = MsgPayload::new(value, payload_data);
    MessageEntry::new(source, incl_epoch, payload)
}

/// Helper to build an update operation using the chunk-aware UpdateBuilder.
///
/// Accepts messages and chunk transitions. The builder validates chunks
/// against its internal pending input tracking.
pub(crate) fn build_update_operation(
    seq_no: u64,
    messages: Vec<MessageEntry>,
    chunks: &[ChunkTransition],
    initial_state: &EeAccountState,
    snark_state: &SnarkAccountState,
    ee: &SimpleExecutionEnvironment,
) -> (UpdateOperationData, Vec<Vec<u8>>, SnarkPrivateInput) {
    let vinput = EeVerificationInput::new(ee, &[], &[]);

    let mut builder =
        UpdateBuilder::new(seq_no, snark_state.clone(), initial_state.clone(), vinput)
            .expect("create builder");

    builder.add_messages(messages).expect("add messages");

    for chunk in chunks {
        builder
            .accept_chunk_transition(chunk)
            .expect("accept chunk should succeed");
    }

    let snark_priv = builder
        .build_private_input()
        .expect("build_private_input should succeed");
    let (op, coinputs) = builder.build().expect("build should succeed");
    (op, coinputs, snark_priv)
}

/// Creates a simple [`ChunkTransition`] from deposits and outputs.
pub(crate) fn simple_chunk(
    parent: Hash,
    tip: Hash,
    deposits: Vec<SubjectDepositData>,
    outputs: ExecOutputs,
) -> ChunkTransition {
    let mut inputs = ExecInputs::new_empty();
    for d in deposits {
        inputs.add_subject_deposit(d);
    }
    ChunkTransition::new(parent, tip, inputs, outputs)
}

/// Creates a [`ChunkTransition`] for testing (thin wrapper).
pub(crate) fn create_chunk_transition(
    parent: Hash,
    tip: Hash,
    inputs: ExecInputs,
    outputs: ExecOutputs,
) -> ChunkTransition {
    ChunkTransition::new(parent, tip, inputs, outputs)
}

/// Wraps chunk transitions into [`ChunkInput`]s with empty proofs.
///
/// The always-accept predicate (type ID `0x01`) will pass with any proof bytes.
pub(crate) fn make_chunk_inputs(chunks: &[ChunkTransition]) -> Vec<ChunkInput> {
    chunks
        .iter()
        .map(|c| ChunkInput::new(c.clone(), vec![]))
        .collect()
}

/// Verifies an update through the full verified path with chunk proof checking.
///
/// Constructs a [`SnarkPrivateInput`] from the operation data and initial state,
/// wraps chunk transitions into [`ChunkInput`]s, and delegates to the EE
/// `verify_and_process_update`.
pub(crate) fn verify_with_chunks(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    coinputs: &[Vec<u8>],
    chunks: &[ChunkTransition],
    ee: &SimpleExecutionEnvironment,
) -> ProgramResult<(), EnvError> {
    let pub_params = UpdateProofPubParams::new(
        ProofState::new(Hash::default(), 0),
        ProofState::new(Hash::default(), operation.processed_messages().len() as u64),
        operation.processed_messages().to_vec(),
        operation.ledger_refs().clone(),
        operation.outputs().clone(),
        operation.extra_data().to_vec(),
    );

    let coinputs_typed: Vec<Coinput> = coinputs.iter().map(|v| Coinput::new(v.clone())).collect();

    let snark_priv =
        SnarkPrivateInput::new(pub_params, initial_state.as_ssz_bytes(), coinputs_typed);

    let chunk_inputs = make_chunk_inputs(chunks);
    let ee_priv = EePrivateInput::new(vec![], vec![], chunk_inputs);
    strata_ee_acct_runtime::verify_and_process_update(ee, ee_priv, snark_priv)
}

/// Verifies an update through the full verified path using a pre-built
/// [`SnarkPrivateInput`].
///
/// Wraps chunk transitions into [`ChunkInput`]s and delegates to the EE
/// `verify_and_process_update`.
pub(crate) fn verify_with_private_input(
    snark_priv: &SnarkPrivateInput,
    chunks: &[ChunkTransition],
    ee: &SimpleExecutionEnvironment,
) -> ProgramResult<(), EnvError> {
    let chunk_inputs = make_chunk_inputs(chunks);
    let ee_priv = EePrivateInput::new(vec![], vec![], chunk_inputs);
    strata_ee_acct_runtime::verify_and_process_update(ee, ee_priv, snark_priv.clone())
}

/// Asserts that the verified path succeeds using the builder's private input.
pub(crate) fn assert_verified_path_succeeds(
    snark_priv: &SnarkPrivateInput,
    chunks: &[ChunkTransition],
    ee: &SimpleExecutionEnvironment,
) {
    verify_with_private_input(snark_priv, chunks, ee).expect("verified path should succeed");
}

/// Asserts that the verified path with chunks succeeds.
pub(crate) fn assert_verified_chunks_succeed(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    coinputs: &[Vec<u8>],
    chunks: &[ChunkTransition],
    ee: &SimpleExecutionEnvironment,
) {
    verify_with_chunks(initial_state, operation, coinputs, chunks, ee)
        .expect("verified path with chunks should succeed");
}

/// Creates an [`EeVerificationState`] for testing.
pub(crate) fn create_vstate<'a>(
    ee: &'a SimpleExecutionEnvironment,
    initial_state: &EeAccountState,
    expected_outputs: UpdateOutputs,
) -> EeVerificationState<'a, SimpleExecutionEnvironment> {
    EeVerificationState::new_from_state(ee, initial_state, expected_outputs, &[], &[])
}
