//! Common test utilities for integration tests.

#![allow(unreachable_pub, reason = "test utilities")]
#![allow(dead_code, reason = "utilities used by different test files")]

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SubjectId};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{ChainSegmentBuilder, UpdateBuilder};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState, ExecHeader, PendingInputEntry};
use strata_ee_chain_types::{BlockInputs, SubjectDepositData};
use strata_msg_fmt::Msg as MsgTrait;
use strata_simple_ee::{
    SimpleBlockBody, SimpleExecutionEnvironment, SimpleHeader, SimpleHeaderIntrinsics,
    SimplePartialState,
};
use strata_snark_acct_types::{MessageEntry, UpdateOperationData};

/// Helper to assert that both update application paths yield the same final state.
///
/// This tests that `verify_and_apply_update_operation` and
/// `apply_update_operation_unconditionally` produce identical results.
pub fn assert_update_paths_match(
    initial_state: &EeAccountState,
    operation: &UpdateOperationData,
    shared_private: &strata_ee_acct_runtime::SharedPrivateInput,
    coinputs: &[Vec<u8>],
    ee: &SimpleExecutionEnvironment,
) {
    // Apply with verification
    let mut verified_state = initial_state.clone();

    strata_ee_acct_runtime::verify_and_apply_update_operation(
        &mut verified_state,
        operation,
        coinputs.iter().map(|v| v.as_slice()),
        shared_private,
        ee,
    )
    .expect("verify_and_apply should succeed");

    // Apply unconditionally
    let mut unconditional_state = initial_state.clone();
    let input_data: strata_snark_acct_types::UpdateInputData = operation.clone().into();
    strata_ee_acct_runtime::apply_update_operation_unconditionally(
        &mut unconditional_state,
        &input_data,
    )
    .expect("apply_unconditionally should succeed");

    // Compare the two states
    assert_eq!(
        verified_state, unconditional_state,
        "Verified and unconditional application paths should yield identical states"
    );
}

/// Creates a simple initial state for testing.
pub(crate) fn create_initial_state() -> (EeAccountState, SimplePartialState, SimpleHeader) {
    let ee_state =
        EeAccountState::new([0u8; 32], BitcoinAmount::from(0u64), Vec::new(), Vec::new());

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
