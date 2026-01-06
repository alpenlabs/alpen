//! Common test utilities for integration tests.

#![allow(unreachable_pub, reason = "test utilities")]
#![allow(dead_code, reason = "utilities used by different test files")]

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{ChainSegmentBuilder, UpdateBuilder, UpdateTransitionData};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState, ExecHeader, PendingInputEntry};
use strata_ee_chain_types::{BlockInputs, SubjectDepositData};
use strata_msg_fmt::Msg as MsgTrait;
use strata_simple_ee::{
    SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeader, SimpleHeaderIntrinsics,
    SimplePartialState,
};
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateOperationData};
use tree_hash::{Sha256Hasher, TreeHash};

/// Converts a full UpdateOperationData into UpdateTransitionData.
///
/// This is used for testing that a single transition covering the entire update
/// is equivalent to unconditional application. The SharedPrivateInput remains
/// unchanged since it contains all the blocks to execute.
pub fn update_to_transition_data(
    operation: &UpdateOperationData,
    initial_state: &EeAccountState,
) -> UpdateTransitionData {
    // Compute initial state hash - this becomes the transition's prev_state
    let initial_state_hash = TreeHash::<Sha256Hasher>::tree_hash_root(initial_state);
    let prev_state = ProofState::new(Hash::from(initial_state_hash.0), 0);

    // A single transition processes all messages, outputs, and blocks
    UpdateTransitionData::new(
        prev_state,
        operation.new_state(),
        operation.processed_messages().to_vec(),
        operation.outputs().clone(),
        operation.extra_data().to_vec(),
    )
}

/// Creates a simple initial state for testing.
pub(crate) fn create_initial_state() -> (EeAccountState, SimplePartialState, SimpleHeader) {
    let ee_state = EeAccountState::new(
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
    let mut inputs = BlockInputs::new_empty();
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

/// Helper to build a chain segment with multiple blocks (one block per deposit).
///
/// Unlike `build_chain_segment_with_deposits` which creates a single block consuming all deposits,
/// this function creates one block for each deposit.
///
/// Returns the segment along with the final execution state and header.
pub(crate) fn build_chain_segment_with_multiple_blocks(
    ee: SimpleExecutionEnvironment,
    initial_state: SimplePartialState,
    initial_header: SimpleHeader,
    deposits: Vec<SubjectDepositData>,
) -> (CommitChainSegment, SimplePartialState, SimpleHeader) {
    let pending_inputs: Vec<PendingInputEntry> = deposits
        .iter()
        .map(|d| PendingInputEntry::Deposit(d.clone()))
        .collect();

    let mut builder =
        ChainSegmentBuilder::new(ee, initial_state, initial_header.clone(), pending_inputs);

    // Create one block per deposit
    for (i, deposit) in deposits.into_iter().enumerate() {
        let body = SimpleBlockBody::new(vec![]);
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(deposit);

        let parent_blkid = builder.current_header().compute_block_id();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid,
            index: initial_header.index() + 1 + (i as u64),
        };

        builder
            .append_block_body(&intrinsics, body, inputs)
            .expect("append block should succeed");
    }

    // Extract final state and header before building
    let final_state = builder.current_state().clone();
    let final_header = builder.current_header().clone();
    let segment = builder.build();

    (segment, final_state, final_header)
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

/// Helper to build a transition operation for testing.
///
/// This is a convenience wrapper that builds an UpdateOperationData and
/// converts it to UpdateTransitionData. Use this when you want to
/// test transition-based processing.
pub(crate) fn build_transition_data(
    seq_no: u64,
    messages: Vec<MessageEntry>,
    segments: Vec<CommitChainSegment>,
    initial_state: &EeAccountState,
    prev_header: &SimpleHeader,
    prev_partial_state: &SimplePartialState,
) -> (
    UpdateTransitionData,
    strata_ee_acct_runtime::SharedPrivateInput,
    Vec<Vec<u8>>,
) {
    let (operation, shared_private, coinputs) = build_update_operation(
        seq_no,
        messages,
        segments,
        initial_state,
        prev_header,
        prev_partial_state,
    );
    let transition_data = update_to_transition_data(&operation, initial_state);
    (transition_data, shared_private, coinputs)
}

/// Assert that transition-based verification and unconditional update application produce identical
/// results.
///
/// This helper function is used by tests to verify equivalence between:
/// 1. Transition-based processing (with verification)
/// 2. Unconditional update application
pub(crate) fn assert_update_paths_match<E: strata_ee_acct_types::ExecutionEnvironment>(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    shared_private: &strata_ee_acct_runtime::SharedPrivateInput,
    coinputs: &[Vec<u8>],
    ee: &E,
) {
    // Convert to transition and apply with verification
    let transition_data = update_to_transition_data(operation, initial_state);
    let mut state_transition = initial_state.clone();
    strata_ee_acct_runtime::verify_and_apply_update_transition(
        &mut state_transition,
        &transition_data,
        coinputs.iter().map(|v| v.as_slice()),
        shared_private,
        ee,
    )
    .expect("transition verification should succeed");

    // Apply unconditionally
    let mut state_unconditional = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = operation.clone().into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut state_unconditional,
        &input_data,
    )
    .expect("unconditional application should succeed");

    // Assert both paths match
    assert_eq!(
        state_transition, state_unconditional,
        "Transition-based and unconditional paths should produce identical states"
    );
}
