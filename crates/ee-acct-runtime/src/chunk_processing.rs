//! Chunk-level processing logic.
//!
//! This module contains the main function [`verify_and_apply_chunk_operation`]
//! which verifies and applies a single chunk operation within a proof.
//! Chunk operations are subsets of full update operations - they handle
//! EVM block execution but not DA verification (which is handled by the outer proof).

use strata_codec::decode_buf_exact;
use strata_ee_acct_types::{
    EeAccountState, EnvError, EnvResult, ExecutionEnvironment, UpdateExtraData,
};
use strata_snark_acct_types::UpdateOutputs;

use crate::{
    chunk_operation::ChunkOperationData,
    exec_processing::process_segments,
    private_input::SharedPrivateInput,
    update_processing::{
        MsgData, apply_extra_data, apply_message, make_inp_err_indexer, verify_acct_state_matches,
    },
    verification_state::{ChunkVerificationState, InputTracker, PendingCommit},
};

/// Common data from various sources passed around together.
struct SharedData<'v> {
    operation: &'v ChunkOperationData,
    extra: &'v UpdateExtraData,
    shared_private: &'v SharedPrivateInput,
}

impl<'v> SharedData<'v> {
    #[expect(dead_code, reason = "for future use")]
    fn outputs(&self) -> &'v UpdateOutputs {
        self.operation.outputs()
    }

    fn extra(&self) -> &'v UpdateExtraData {
        self.extra
    }

    fn private_input(&self) -> &'v SharedPrivateInput {
        self.shared_private
    }
}

/// Verify if a chunk operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
///
/// This is chunk-scoped - DA verification is handled by the outer proof.
pub fn verify_and_apply_chunk_operation<'i>(
    astate: &mut EeAccountState,
    operation: &ChunkOperationData,
    coinputs: impl IntoIterator<Item = &'i [u8]>,
    shared_private: &SharedPrivateInput,
    ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    let mut coinp_iter = coinputs.into_iter().fuse();

    // Basic parsing/handling for things.
    // TODO clean this up a little
    let extra =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;
    let shared = SharedData {
        operation,
        extra: &extra,
        shared_private,
    };

    // 1. Process each message in order.
    let mut cvstate = ChunkVerificationState::new_from_state(astate);
    let processed_messages = operation.processed_messages();

    for (i, inp) in processed_messages.iter().enumerate() {
        // Get the corresponding coinput for this message.
        let Some(coinp) = coinp_iter.next() else {
            return Err(EnvError::MismatchedCoinputCnt);
        };

        let Some(msg) = MsgData::from_entry(inp).ok() else {
            // Don't allow coinputs if we're ignoring it.
            if !coinp.is_empty() {
                return Err(EnvError::MismatchedCoinputIdx(i));
            }

            // Other type or invalid message, skip.
            continue;
        };

        // Process the coinput and message, probably verifying them against each
        // other and inserting entries in the verification state for later.
        handle_coinput_for_message(&mut cvstate, astate, &msg, coinp, &shared, ee)
            .map_err(make_inp_err_indexer(i))?;

        // Then apply the message.  This doesn't rely on the private coinput.
        apply_message(astate, &msg).map_err(make_inp_err_indexer(i))?;
    }

    // Make sure there are no more leftover coinputs we haven't recognized.
    if coinp_iter.next().is_some() {
        return Err(EnvError::MismatchedCoinputCnt);
    }

    // 2. Ensure that the accumulated effects match the final state.
    verify_accumulated_state(&mut cvstate, astate, &shared, ee)?;
    cvstate.check_obligations()?;

    // 3. Apply the extra data.
    apply_extra_data(astate, &extra)?;

    // 4. Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

fn handle_coinput_for_message(
    _cvstate: &mut ChunkVerificationState,
    _astate: &EeAccountState,
    _msg: &MsgData,
    coinp: &[u8],
    _shared: &SharedData<'_>,
    _ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    // Actually, new plan, we don't need any message coinputs right now.
    if !coinp.is_empty() {
        return Err(EnvError::MalformedCoinput);
    }

    Ok(())
}

fn verify_accumulated_state(
    cvstate: &mut ChunkVerificationState,
    astate: &EeAccountState,
    shared: &SharedData<'_>,
    ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    // Temporary measure: interpret the new tip blkid in the extra data as a
    // commit, but only if it changed from the current tip.
    if *shared.extra.new_tip_blkid() != astate.last_exec_blkid() {
        cvstate.add_pending_commit(PendingCommit::new(*shared.extra.new_tip_blkid()));
    }

    // 1. Validate that we got chain segments corresponding to the pending commits.
    if cvstate.pending_commits().len() != shared.private_input().commit_data().len() {
        return Err(EnvError::MismatchedChainSegment);
    }

    // 2. Verify segments against the accumulated state.
    let mut input_tracker = InputTracker::new(astate.pending_inputs());
    process_segments(
        cvstate,
        &mut input_tracker,
        shared.shared_private.commit_data(),
        shared.private_input().raw_partial_pre_state(),
        shared.private_input().raw_prev_header(),
        ee,
    )?;

    // 3. Check that the inputs we consumed match the number we're syaing were
    // consumed.
    if input_tracker.consumed() != *shared.extra().processed_inputs() as usize {
        return Err(EnvError::InvalidBlock);
    }

    // A. Check balance changes are consistent.
    // TODO

    // C. Check ledger references (DA) match what was ultimately computed.
    // TODO figure out DA plan

    Ok(())
}
