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
use strata_ee_acct_types::{
    CommitChainSegment, DecodedEeMessage, EeAccountState, EnvError, EnvResult,
    ExecutionEnvironment, PendingInputEntry, UpdateExtraData,
};
use strata_ee_chain_types::SubjectDepositData;
use strata_snark_acct_types::{LedgerRefs, MessageEntry, UpdateOperationData, UpdateOutputs};

use crate::{
    exec_processing::{apply_commit, verify_chain_segment},
    private_input::SharedPrivateInput,
    verification_state::UpdateVerificationState,
};

/// Common data from various sources passed around together.
struct SharedData<'v> {
    operation: &'v UpdateOperationData,
    extra: &'v UpdateExtraData,
    shared_private: &'v SharedPrivateInput,
}

impl<'v> SharedData<'v> {
    pub fn seq_no(&self) -> u64 {
        self.operation.seq_no()
    }

    pub fn ledger_refs(&self) -> &'v LedgerRefs {
        self.operation.ledger_refs()
    }

    pub fn outputs(&self) -> &'v UpdateOutputs {
        self.operation.outputs()
    }

    pub fn extra(&self) -> &'v UpdateExtraData {
        self.extra
    }

    pub fn private_input(&self) -> &'v SharedPrivateInput {
        self.shared_private
    }
}

/// Meta fields extracted from a message.
struct MsgMeta {
    source: AccountId,
    incl_epoch: u32,
    value: BitcoinAmount,
}

/// Data unique to a message.
struct MsgData {
    meta: MsgMeta,
    message: DecodedEeMessage,
}

impl MsgData {
    fn from_entry(m: &MessageEntry) -> Option<Self> {
        let message = DecodedEeMessage::decode_raw(m.payload_buf())?;
        let meta = MsgMeta {
            source: m.source(),
            incl_epoch: m.incl_epoch(),
            value: m.payload_value(),
        };

        Some(Self { meta, message })
    }
}

/// Verify if an update operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
///
/// This accepts the update operation data as an argument.
pub fn verify_and_apply_update_operation<'i>(
    state: &mut EeAccountState,
    operation: &UpdateOperationData,
    coinputs: impl IntoIterator<Item = &'i [u8]>,
    shared_private: &SharedPrivateInput,
    ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    let mut coinp_iter = coinputs.into_iter().fuse();

    // Basic parsing/handling for things.
    let extra = UpdateExtraData::decode(operation.extra_data()).map_err(|_| EnvError::Decode)?;
    let shared = SharedData {
        operation,
        extra: &extra,
        shared_private,
    };

    // 1. Process each message in order.
    let mut vstate = UpdateVerificationState::new_from_state(state);
    for (inp, coinp) in operation.processed_messages().iter().zip(&mut coinp_iter) {
        let Some(msg) = MsgData::from_entry(inp) else {
            // Other type or invalid message, skip.
            continue;
        };

        // Process the coinput and message, probably verifying them against each
        // other and inserting entries in the verification state for later.
        handle_coinput_for_message(&mut vstate, state, &msg, coinp, &shared, ee)?;

        // Then apply the message.  This doesn't rely on the private coinput.
        apply_message(state, &msg, &extra)?;
    }

    // Make sure there are no more leftover coinputs we haven't recognized.
    if coinp_iter.next().is_some() {
        return Err(EnvError::ExtraCoinputs);
    }

    // 2. Ensure that the accumulated effects match the final state.
    verify_accumulated_state(&mut vstate, state, &shared)?;

    // 3. Apply final changes.
    apply_final_update_changes(state, &extra)?;

    // 4. Verify the final EE state matches `new_state`.
    // TODO

    Ok(())
}

fn handle_coinput_for_message(
    vstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    msg: &MsgData,
    coinp: &[u8],
    shared: &SharedData<'_>,
    ee: &impl ExecutionEnvironment,
) -> EnvResult<()> {
    // Actually, new plan, we don't need any message coinputs right now.
    if !coinp.is_empty() {
        return Err(EnvError::MalformedCoinput);
    }

    Ok(())
}

fn verify_accumulated_state(
    vstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    shared: &SharedData<'_>,
) -> EnvResult<()> {
    // 1. Process each block, tracking inputs and outputs.

    // 2. Check balance changes are consistent.

    // 3. Check outputs match what's claimed.

    // 4. Check ledger references (DA) match what was ultimately computed.

    // TODO
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
    operation: &UpdateOperationData,
) -> EnvResult<()> {
    let extra = UpdateExtraData::decode(operation.extra_data()).map_err(|_| EnvError::Decode)?;

    // 1. Apply the changes from the messages.
    for inp in operation.processed_messages().iter() {
        let Some(msg) = MsgData::from_entry(inp) else {
            continue;
        };

        apply_message(astate, &msg, &extra)?;
    }

    // 2. Apply the final update changes.
    apply_final_update_changes(astate, &extra)?;

    // 3. Verify the final EE state matches `new_state`.
    // TODO

    Ok(())
}

fn apply_message(
    astate: &mut EeAccountState,
    msg: &MsgData,
    extra: &UpdateExtraData,
) -> EnvResult<()> {
    // TODO dispatch to handler depending on message type

    match &msg.message {
        DecodedEeMessage::Deposit(data) => {
            let deposit_data = SubjectDepositData::new(data.dest_subject(), msg.meta.value);
            astate.add_tracked_balance(msg.meta.value);
            astate.add_pending_input(PendingInputEntry::Deposit(deposit_data));
        }

        DecodedEeMessage::SubjTransfer(_data) => {
            astate.add_tracked_balance(msg.meta.value);
            // TODO
        }

        DecodedEeMessage::Commit(_data) => {
            if !msg.meta.value.is_zero() {
                // TODO maybe do something here, not sure
            }

            // TODO figure out what to do with this
        }
    }

    Ok(())
}

fn apply_final_update_changes(
    state: &mut EeAccountState,
    extra: &UpdateExtraData,
) -> EnvResult<()> {
    // 1. Update final execution head block.

    Ok(())
}
