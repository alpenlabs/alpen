//! High-level procedures for processing updates.
//!
//! There are two main functions here.  The first is
//! [`verify_and_apply_update_operation`], which is what we use in the proof to
//! carefully check and update the account state.  This relies on an execution
//! environment impl to be able to check the things.
//!
//! The second is [`apply_update_operation_unconditionally`], which is used
//! outside the proof, after verifying the proof, to update our view of the
//! state, presumably with information extracted from DA.  This does not require
//! understanding the execution environment.

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_codec::decode_buf_exact;
use strata_ee_acct_types::{
    DecodedEeMessageData, EeAccountState, EnvError, EnvResult, ExecutionEnvironment,
    MessageDecodeResult, PendingInputEntry, UpdateExtraData,
};
use strata_ee_chain_types::SubjectDepositData;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, UpdateInputData, UpdateOperationData, UpdateOutputs,
};

use crate::{
    exec_processing::process_segments,
    private_input::SharedPrivateInput,
    verification_state::{InputTracker, PendingCommit, UpdateVerificationState},
};

/// Common data from various sources passed around together.
struct SharedData<'v> {
    operation: &'v UpdateOperationData,
    extra: &'v UpdateExtraData,
    shared_private: &'v SharedPrivateInput,
}

impl<'v> SharedData<'v> {
    #[expect(dead_code, reason = "for future use")]
    fn seq_no(&self) -> u64 {
        self.operation.seq_no()
    }

    #[expect(dead_code, reason = "for future use")]
    fn ledger_refs(&self) -> &'v LedgerRefs {
        self.operation.ledger_refs()
    }

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

/// Meta fields extracted from a message.
pub(crate) struct MsgMeta {
    #[expect(dead_code, reason = "for future use")]
    pub(crate) source: AccountId,
    #[expect(dead_code, reason = "for future use")]
    pub(crate) incl_epoch: u64,
    pub(crate) value: BitcoinAmount,
}

/// Decoded message and its metadata.
pub(crate) struct MsgData {
    pub(crate) meta: MsgMeta,
    pub(crate) message: DecodedEeMessageData,
}

impl MsgData {
    pub(crate) fn from_entry(m: &MessageEntry) -> MessageDecodeResult<Self> {
        let message = DecodedEeMessageData::decode_raw(m.payload_buf())?;
        let meta = MsgMeta {
            source: m.source(),
            incl_epoch: m.incl_epoch(),
            value: m.payload_value(),
        };

        Ok(Self { meta, message })
    }
}

/// Verify if an update operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
///
/// This accepts the update operation data as an argument.
pub fn verify_and_apply_update_operation<'i>(
    astate: &mut EeAccountState,
    operation: &UpdateOperationData,
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
    let mut uvstate = UpdateVerificationState::new_from_state(astate);
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
        handle_coinput_for_message(&mut uvstate, astate, &msg, coinp, &shared, ee)
            .map_err(make_inp_err_indexer(i))?;

        // Then apply the message.  This doesn't rely on the private coinput.
        apply_message(astate, &msg, &extra).map_err(make_inp_err_indexer(i))?;
    }

    // Make sure there are no more leftover coinputs we haven't recognized.
    if coinp_iter.next().is_some() {
        return Err(EnvError::MismatchedCoinputCnt);
    }

    // 2. Ensure that the accumulated effects match the final state.
    verify_accumulated_state(&mut uvstate, astate, &shared, ee)?;
    uvstate.check_obligations()?;

    // 3. Apply final changes.
    apply_final_update_changes(astate, &extra)?;

    // 4. Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

fn handle_coinput_for_message(
    _uvstate: &mut UpdateVerificationState,
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
    uvstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    shared: &SharedData<'_>,
    ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    // Temporary measure: interpret the new tip blkid in the extra data as a
    // commit, but only if it changed from the current tip.
    if *shared.extra.new_tip_blkid() != astate.last_exec_blkid() {
        uvstate.add_pending_commit(PendingCommit::new(*shared.extra.new_tip_blkid()));
    }

    // 1. Validate that we got chain segments corresponding to the pending commits.
    if uvstate.pending_commits().len() != shared.private_input().commit_data().len() {
        return Err(EnvError::MismatchedChainSegment);
    }

    // 2. Verify segments against the accumulated state.
    let mut input_tracker = InputTracker::new(astate.pending_inputs());
    process_segments(
        uvstate,
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

/// Applies the effects of an update, but does not check the messages.  It's
/// assumed we have a proof attesting to the validity that transitively attests
/// to this.
///
/// This is used in clients after they have a proof for an update to reconstruct
/// the actual state proven by the proof.
pub fn apply_update_operation_unconditionally(
    astate: &mut EeAccountState,
    operation: &UpdateInputData,
) -> EnvResult<()> {
    let extra =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;

    // 1. Apply the changes from the messages.
    for (i, inp) in operation.processed_messages().iter().enumerate() {
        let Some(msg) = MsgData::from_entry(inp).ok() else {
            continue;
        };

        apply_message(astate, &msg, &extra).map_err(make_inp_err_indexer(i))?;
    }

    // 2. Apply the final update changes.
    apply_final_update_changes(astate, &extra)?;

    // 3. Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

/// Applies state changes from the message.
pub(crate) fn apply_message(
    astate: &mut EeAccountState,
    msg: &MsgData,
    _extra: &UpdateExtraData,
) -> EnvResult<()> {
    if !msg.meta.value.is_zero() {
        astate.add_tracked_balance(msg.meta.value);
    }

    match &msg.message {
        DecodedEeMessageData::Deposit(data) => {
            let deposit_data = SubjectDepositData::new(*data.dest_subject(), msg.meta.value);
            astate.add_pending_input(PendingInputEntry::Deposit(deposit_data));
        }

        DecodedEeMessageData::SubjTransfer(_data) => {
            // TODO
        }

        DecodedEeMessageData::Commit(_data) => {
            // Just ignore this one for now because we're not handling it.
            // TODO improve
        }
    }

    Ok(())
}

/// Applies the final changes to the account state.
///
/// This just updates the exec tip blkid and removes pending entries.
fn apply_final_update_changes(
    state: &mut EeAccountState,
    extra: &UpdateExtraData,
) -> EnvResult<()> {
    // 1. Update final execution head block.
    state.set_last_exec_blkid(*extra.new_tip_blkid());

    // 2. Update queues.
    state.remove_pending_inputs(*extra.processed_inputs() as usize);
    state.remove_pending_fincls(*extra.processed_fincls() as usize);

    Ok(())
}

fn verify_acct_state_matches(
    _astate: &EeAccountState,
    _exp_new_state: &[u8; 32],
) -> Result<(), EnvError> {
    // TODO use SSZ hash_tree_root
    Ok(())
}

fn maybe_index_inp_err(e: EnvError, idx: usize) -> EnvError {
    match e {
        EnvError::MalformedCoinput => EnvError::MalformedCoinputIdx(idx),
        EnvError::MismatchedCoinput => EnvError::MismatchedCoinputIdx(idx),
        EnvError::InconsistentCoinput => EnvError::InconsistentCoinputIdx(idx),
        _ => e,
    }
}

fn make_inp_err_indexer(idx: usize) -> impl Fn(EnvError) -> EnvError {
    move |e| maybe_index_inp_err(e, idx)
}
