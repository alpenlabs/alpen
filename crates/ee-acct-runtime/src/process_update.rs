//! High-level procedures for processing updates.
//!
//! There are two main functions here.  The first is
//! [`verify_and_apply_update_operation`], which is what we use in the proof to
//! carefully check and update the account state.
//!
//! The second is [`apply_update_operation_unconditionally`], which is used
//! outside the proof, after verifying the proof, to update our view of the
//! state, presumably with information extracted from DA.

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_ee_acct_types::{
    CommitCoinput, DecodedEeMessage, EeAccountState, EnvError, EnvResult, ExecutionEnvironment,
};
use strata_snark_acct_types::{MessageEntry, UpdateOperationData};

use crate::{
    exec_processing::{apply_commit, verify_commit},
    verification_state::UpdateVerificationState,
};

/// Verify if an update operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
pub fn verify_and_apply_update_operation<'i, E: ExecutionEnvironment>(
    state: &mut EeAccountState,
    operation: &UpdateOperationData,
    coinputs: impl IntoIterator<Item = &'i [u8]>,
    ee: &E,
) -> EnvResult<()> {
    let mut coinp_iter = coinputs.into_iter();

    // 1. Process each message in order.
    let mut vstate = UpdateVerificationState::new_from_state(state);
    for (inp, coinp) in operation.processed_messages().iter().zip(&mut coinp_iter) {
        let Some((meta, eem)) = parse_input(&inp) else {
            // Other type or invalid message, skip.
            continue;
        };

        // Verify the message.  This relies on the private coinput.
        verify_message(&mut vstate, state, &eem, &meta, coinp, operation, ee)?;

        // Then apply the message.  This doesn't rely on the private coinput.
        apply_message(state, &eem, &meta, operation)?;
    }

    // Make sure there are no more leftover coinputs we haven't recognized.
    if coinp_iter.next().is_some() {
        return Err(EnvError::ExtraCoinputs);
    }

    // 2. Ensure that the accumulated effects match the final state.
    verify_accumulated_state(&mut vstate, state, &operation)?;

    // 3. Apply final changes.
    apply_final_update_changes(state, &operation)?;

    Ok(())
}

fn verify_message<E: ExecutionEnvironment>(
    vstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    msg: &DecodedEeMessage,
    meta: &MsgMeta,
    coinp: &[u8],
    op: &UpdateOperationData,
    ee: &E,
) -> EnvResult<()> {
    // TODO dispatch to handler depending on message type

    match msg {
        DecodedEeMessage::Deposit(deposit_msg_data) => {
            // No coinputs allowed for this one.
            if !coinp.is_empty() {
                return Err(EnvError::MalformedCoinput);
            }
        }

        DecodedEeMessage::SubjTransfer(subj_transfer_msg_data) => {
            // No coinputs allowed for this one.
            if !coinp.is_empty() {
                return Err(EnvError::MalformedCoinput);
            }
        }

        DecodedEeMessage::Commit(commit_msg_data) => {
            let Some(cc) = CommitCoinput::decode_raw(coinp) else {
                return Err(EnvError::MalformedCoinput);
            };

            verify_commit(vstate, astate, commit_msg_data, &cc, ee)?;
        }
    }

    Ok(())
}

fn verify_accumulated_state(
    vstate: &mut UpdateVerificationState,
    astate: &EeAccountState,
    op: &UpdateOperationData,
) -> EnvResult<()> {
    // 1. Check balance changes are consistent.

    // 2. Check ledger references (DA) match what was demanded.

    // 3. Check outputs match what's claimed.

    // TODO
    Ok(())
}

/// Applies the effects of an update, but does not check the messages.  It's
/// assumed we have a proof attesting to the validity that transitively attests
/// to this.
///
/// This is used in clients after they have a proof for an update to reconstruct
/// the actual state proven by the proof.
pub fn apply_update_operation_unconditionally<E: ExecutionEnvironment>(
    astate: &mut EeAccountState,
    operation: &UpdateOperationData,
) -> EnvResult<()> {
    // 1. Apply the changes from the messages.
    for inp in operation.processed_messages().iter() {
        let Some((meta, eem)) = parse_input(&inp) else {
            continue;
        };

        apply_message(astate, &eem, &meta, operation)?;
    }

    // 2. Apply the final update changes.
    apply_final_update_changes(astate, operation)?;

    // TODO
    Ok(())
}

fn apply_message(
    astate: &mut EeAccountState,
    msg: &DecodedEeMessage,
    meta: &MsgMeta,
    op: &UpdateOperationData,
) -> EnvResult<()> {
    // TODO dispatch to handler depending on message type

    match msg {
        DecodedEeMessage::Deposit(deposit_msg_data) => {
            // TODO
        }

        DecodedEeMessage::SubjTransfer(subj_transfer_msg_data) => {
            // TODO
        }

        DecodedEeMessage::Commit(commit_msg_data) => {
            apply_commit(astate, commit_msg_data)?;
        }
    }

    Ok(())
}

fn apply_final_update_changes(
    state: &mut EeAccountState,
    op: &UpdateOperationData,
) -> EnvResult<()> {
    // 1. Update final execution head block.

    Ok(())
}

/// Meta fields extracted from a message.
struct MsgMeta {
    source: AccountId,
    incl_epoch: u32,
    value: BitcoinAmount,
}

fn parse_input(m: &MessageEntry) -> Option<(MsgMeta, DecodedEeMessage)> {
    let eem = DecodedEeMessage::decode_raw(m.payload_buf())?;
    let meta = MsgMeta {
        source: m.source(),
        incl_epoch: m.incl_epoch(),
        value: m.payload_value(),
    };
    Some((meta, eem))
}
