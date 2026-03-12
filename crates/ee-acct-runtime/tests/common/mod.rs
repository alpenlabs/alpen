//! Common test utilities for integration tests.

#![allow(unreachable_pub, reason = "test utilities")]
#![allow(dead_code, reason = "utilities used by different test files")]

use rkyv::rancor::Error as RkyvError;
use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ArchivedEePrivateInput, ChunkInput, EePrivateInput, EeSnarkAccountProgram, EeVerificationInput,
    EeVerificationState, UpdateBuilder,
};
use strata_ee_acct_types::{DecodedEeMessageData, EeAccountState, EnvError, UpdateExtraData};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs, SubjectDepositData};
use strata_msg_fmt::Msg as MsgTrait;
use strata_predicate::PredicateKey;
use strata_simple_ee::SimpleExecutionEnvironment;
use strata_snark_acct_runtime::{
    ArchivedPrivateInput as ArchivedSnarkPrivateInput, Coinput, IInnerState, InputMessage,
    PrivateInput as SnarkPrivateInput, ProgramResult, SnarkAccountProgram,
};
use strata_snark_acct_types::{
    MessageEntry, ProofState, SnarkAccountState, UpdateManifest, UpdateOperationData,
    UpdateOutputs, UpdateProofPubParams,
};

/// Serializes an [`EePrivateInput`] and a [`SnarkPrivateInput`] with rkyv, then
/// calls `f` with the archived references.
///
/// This helper manages buffer lifetimes so callers don't have to deal with the
/// raw rkyv plumbing.
fn with_archived_inputs<R>(
    ee_priv: &EePrivateInput,
    snark_priv: &SnarkPrivateInput,
    f: impl FnOnce(&ArchivedEePrivateInput, &ArchivedSnarkPrivateInput) -> R,
) -> R {
    let ee_bytes = rkyv::to_bytes::<RkyvError>(ee_priv).expect("rkyv encode EE priv input");
    let snark_bytes =
        rkyv::to_bytes::<RkyvError>(snark_priv).expect("rkyv encode snark priv input");

    // SAFETY: Buffers were just produced by `rkyv::to_bytes` above.
    let archived_ee: &ArchivedEePrivateInput = unsafe { rkyv::access_unchecked(&ee_bytes) };
    let archived_snark: &ArchivedSnarkPrivateInput =
        unsafe { rkyv::access_unchecked(&snark_bytes) };

    f(archived_ee, archived_snark)
}

/// Creates a [`SnarkAccountState`] that matches the given [`EeAccountState`].
///
/// Computes the real state root from the EE state via its `IInnerState` impl.
pub fn make_snark_state(ee_state: &EeAccountState) -> SnarkAccountState {
    SnarkAccountState {
        // Dummy VK since we don't actually care about it in any of these tests.
        update_vk: Vec::new().into(),
        proof_state: ProofState::new(ee_state.compute_state_root(), 0),
        seq_no: 0,
        inbox_mmr: strata_acct_types::Mmr64 {
            entries: 0,
            roots: Default::default(),
        },
    }
}

/// Computes the post-state by manually applying program steps without root checking.
///
/// Used to derive the correct post-state root for test helpers that need to construct
/// [`UpdateProofPubParams`] or [`UpdateManifest`] with real state commitments.
fn compute_post_state(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
) -> EeAccountState {
    let prog = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
    let mut state = initial_state.clone();

    prog.start_update(&mut state).expect("start_update");

    for msg_entry in operation.processed_messages() {
        let inp_msg = InputMessage::<DecodedEeMessageData>::from_msg_entry(msg_entry);
        prog.process_message(&mut state, inp_msg)
            .expect("process_message");
    }

    let extra_data: UpdateExtraData =
        strata_codec::decode_buf_exact(operation.extra_data()).expect("decode extra data");
    prog.pre_finalize_state(&mut state, &extra_data)
        .expect("pre_finalize");
    prog.finalize_state(&mut state, extra_data)
        .expect("finalize");

    state
}

/// Applies an update unconditionally (DA reconstruction path).
pub fn apply_unconditionally(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
) -> ProgramResult<(), EnvError> {
    let mut state = initial_state.clone();

    let post_state = compute_post_state(initial_state, operation);
    let post_root = post_state.compute_state_root();

    let manifest = UpdateManifest::new(
        ProofState::new(post_root, operation.processed_messages().len() as u64),
        operation.extra_data().to_vec(),
        operation.processed_messages().to_vec(),
    );

    let predicate_key = PredicateKey::always_accept();
    strata_ee_acct_runtime::process_update_unconditionally::<SimpleExecutionEnvironment>(
        &mut state,
        &manifest,
        predicate_key,
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
    let pre_root = initial_state.compute_state_root();
    let post_state = compute_post_state(initial_state, operation);
    let post_root = post_state.compute_state_root();

    let pub_params = UpdateProofPubParams::new(
        ProofState::new(pre_root, 0),
        ProofState::new(post_root, operation.processed_messages().len() as u64),
        operation.processed_messages().to_vec(),
        operation.ledger_refs().clone(),
        operation.outputs().clone(),
        operation.extra_data().to_vec(),
    );

    let coinputs_typed: Vec<Coinput> = coinputs.iter().map(|v| Coinput::new(v.clone())).collect();

    let snark_priv =
        SnarkPrivateInput::new(pub_params, initial_state.as_ssz_bytes(), coinputs_typed);

    let ee_priv = EePrivateInput::new(vec![], vec![], vec![]);

    with_archived_inputs(&ee_priv, &snark_priv, |archived_ee, archived_snark| {
        let predicate_key = PredicateKey::always_accept();
        let vinput = EeVerificationInput::new(ee, &predicate_key, archived_ee.chunks(), &[]);

        let program = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
        strata_snark_acct_runtime::verify_and_process_update(&program, archived_snark, vinput)
    })
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
    let predicate_key = PredicateKey::always_accept();
    let vinput = EeVerificationInput::new(ee, &predicate_key, &[], &[]);

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
    let pre_root = initial_state.compute_state_root();
    let post_state = compute_post_state(initial_state, operation);
    let post_root = post_state.compute_state_root();

    let pub_params = UpdateProofPubParams::new(
        ProofState::new(pre_root, 0),
        ProofState::new(post_root, operation.processed_messages().len() as u64),
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

    with_archived_inputs(&ee_priv, &snark_priv, |archived_ee, archived_snark| {
        let predicate_key = PredicateKey::always_accept();
        let vinput = EeVerificationInput::new(ee, &predicate_key, archived_ee.chunks(), &[]);
        let program = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
        strata_snark_acct_runtime::verify_and_process_update(&program, archived_snark, vinput)
    })
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

    with_archived_inputs(&ee_priv, snark_priv, |archived_ee, archived_snark| {
        let predicate_key = PredicateKey::always_accept();
        let vinput = EeVerificationInput::new(ee, &predicate_key, archived_ee.chunks(), &[]);
        let program = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
        strata_snark_acct_runtime::verify_and_process_update(&program, archived_snark, vinput)
    })
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
    chunk_predicate_key: &'a PredicateKey,
    initial_state: &EeAccountState,
    expected_outputs: UpdateOutputs,
) -> EeVerificationState<'a, SimpleExecutionEnvironment> {
    EeVerificationState::new_from_state(
        ee,
        chunk_predicate_key,
        initial_state,
        expected_outputs,
        &[],
        &[],
    )
}
