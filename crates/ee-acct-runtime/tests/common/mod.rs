//! Common test utilities for integration tests.

#![allow(unreachable_pub, reason = "test utilities")]
#![allow(dead_code, reason = "utilities used by different test files")]

use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ChainSegmentBuilder, EeSnarkAccountProgram, EeVerificationInput, SharedPrivateInput,
    UpdateBuilder,
};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState, ExecHeader, PendingInputEntry};
use strata_ee_chain_types::{ExecInputs, SubjectDepositData};
use strata_msg_fmt::Msg as MsgTrait;
use strata_simple_ee::{
    SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeader, SimpleHeaderIntrinsics,
    SimplePartialState,
};
use strata_snark_acct_runtime::{Coinput, ProgramResult, PrivateInput as SnarkPrivateInput};
use strata_snark_acct_types::{
    MessageEntry, ProofState, UpdateManifest, UpdateOperationData, UpdateProofPubParams,
};

use strata_ee_acct_types::EnvError;

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
    shared_private: &SharedPrivateInput,
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

    let vinput = EeVerificationInput::new(ee, &[], shared_private.raw_partial_pre_state());

    let program = EeSnarkAccountProgram::<SimpleExecutionEnvironment>::new();
    strata_snark_acct_runtime::verify_and_process_update(&program, &snark_priv, vinput)
}

/// Asserts that both verified and unconditional paths succeed.
///
/// Only works for tests where the verified path can succeed (e.g., no-segment
/// tests where `new_tip_blkid == initial last_exec_blkid`).
pub fn assert_both_paths_succeed(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    shared_private: &SharedPrivateInput,
    coinputs: &[Vec<u8>],
    ee: &SimpleExecutionEnvironment,
) {
    verify_update(initial_state, operation, shared_private, coinputs, ee)
        .expect("verified path should succeed");

    apply_unconditionally(initial_state, operation).expect("unconditional path should succeed");
}

/// Creates a simple initial state for testing.
pub(crate) fn create_initial_state() -> (EeAccountState, SimplePartialState, SimpleHeader) {
    let ee_state = EeAccountState::new(
        Vec::new(),
        Hash::new([0u8; 32]),
        BitcoinAmount::from(0u64),
        Vec::new(),
        Vec::new(),
    );

    let exec_state = SimplePartialState::new_empty();
    let header = SimpleHeader::genesis();

    (ee_state, exec_state, header)
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

    // Encode the deposit message data
    let deposit_data = DepositMsgData::new(dest);
    let body = encode_to_vec(&deposit_data).expect("encode deposit data");

    // Create properly formatted message
    let msg = OwnedMsg::new(DEPOSIT_MSG_TYPE, body).expect("create message");
    let payload_data = msg.to_vec();

    let payload = MsgPayload::new(value, payload_data);
    MessageEntry::new(source, incl_epoch, payload)
}

/// Helper to build a simple chain segment with deposits.
pub(crate) fn build_chain_segment_with_deposits(
    ee: SimpleExecutionEnvironment,
    initial_state: SimplePartialState,
    initial_header: SimpleHeader,
    deposits: Vec<SubjectDepositData>,
) -> CommitChainSegment {
    let pending_inputs: Vec<PendingInputEntry> = deposits
        .iter()
        .map(|d| PendingInputEntry::Deposit(d.clone()))
        .collect();

    let mut builder =
        ChainSegmentBuilder::new(ee, initial_state, initial_header.clone(), pending_inputs);

    // Create a single block that consumes all deposits
    let body = SimpleBlockBody::new(vec![]);
    let mut inputs = ExecInputs::new_empty();
    for deposit in deposits {
        inputs.add_subject_deposit(deposit);
    }

    let intrinsics = SimpleHeaderIntrinsics {
        parent_blkid: initial_header.compute_block_id(),
        index: initial_header.index() + 1,
    };

    builder
        .append_block_body(&intrinsics, body, inputs)
        .expect("append block should succeed");

    builder.build()
}

/// Helper to build an update operation using UpdateBuilder.
///
/// This properly constructs the UpdateOperationData along with SharedPrivateInput
/// and coinputs, which are all needed for testing.
pub(crate) fn build_update_operation(
    seq_no: u64,
    messages: Vec<MessageEntry>,
    segments: Vec<CommitChainSegment>,
    initial_state: &EeAccountState,
    prev_header: &SimpleHeader,
    prev_partial_state: &SimplePartialState,
) -> (
    UpdateOperationData,
    strata_ee_acct_runtime::SharedPrivateInput,
    Vec<Vec<u8>>,
) {
    let mut builder = UpdateBuilder::new(seq_no, initial_state.clone());

    // Add messages
    for message in messages {
        builder = builder
            .accept_message(message, Vec::new())
            .expect("accept message should succeed");
    }

    // Add segments
    for segment in segments {
        builder = builder.add_segment(segment);
    }

    // Build the operation
    builder
        .build::<SimpleExecutionEnvironment>(initial_state, prev_header, prev_partial_state)
        .expect("build should succeed")
}
