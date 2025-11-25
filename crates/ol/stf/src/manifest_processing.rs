//! ASM manifest processing.

use strata_acct_types::{AccountId, MsgPayload};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_manifest_types::{CheckpointAckLogData, DepositIntentLogData};
use strata_identifiers::{EpochCommitment, L1Height};
use strata_ledger_types::{IL1ViewState, StateAccessor};
use strata_msg_fmt::{Msg, MsgRef, TypeId};
use strata_ol_chain_types_new::{OLL1ManifestContainer, OLL1Update};

use crate::{
    account_processing,
    constants::BRIDGE_GATEWAY_ACCT_ID,
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
};

/// Processes the manifests from a block, which is part of the epoch sealing
/// processing.
///
/// This does NOT check the preseal root.
pub fn process_block_manifests<S: StateAccessor>(
    state: &mut S,
    mf_cont: &OLL1ManifestContainer,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let terminating_epoch = state.l1_view().cur_epoch();

    // 1. Process all the manifests.
    let orig_l1_height = state.l1_view().last_l1_height();
    let mut last = None;

    for (i, mf) in mf_cont.manifests().iter().enumerate() {
        let real_height = orig_l1_height + i as u32;
        last = Some((real_height, mf));
        process_asm_manifest(state, real_height, mf, context)?;
    }

    if let Some((last_height, last_mf)) = last {
        // TODO this is where we would update the header, if we want to keep
        // that as defined in the spec
    }

    // 2. Finally, we can update the epoch to get it ready for the next epoch.
    state.l1_view_mut().set_cur_epoch(terminating_epoch + 1);

    Ok(())
}

fn process_asm_manifest<S: StateAccessor>(
    state: &mut S,
    real_height: L1Height,
    mf: &AsmManifest,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let estate = state.l1_view();

    // 1. Process each of the logs.
    for log in mf.logs() {
        process_asm_log(state, log, real_height, context)?;
    }

    // 2. Accept the manifest into the ASM MMR.
    state.l1_view_mut().append_manifest(real_height, mf.clone());

    Ok(())
}

fn process_asm_log<S: StateAccessor>(
    state: &mut S,
    log: &AsmLogEntry,
    real_height: L1Height,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Try to parse the log as an SPS-52 message.
    let Some(msg) = log.try_as_msg() else {
        // Not a valid message format, skip it.
        return Ok(());
    };

    // Match on the type ID to determine how to process the log.
    match msg.ty() {
        strata_asm_manifest_types::DEPOSIT_INTENT_ASM_LOG_TYPE_ID => {
            // Parse the deposit intent data, skip if it fails to parse.
            let Ok(data) = log.try_into_log::<DepositIntentLogData>() else {
                return Ok(());
            };
            process_deposit_intent_log(state, &data, context)?;
        }

        strata_asm_manifest_types::CHECKPOINT_ACK_ASM_LOG_TYPE_ID => {
            // Parse the checkpoint acknowledgment data, skip if it fails to parse.
            let Ok(data) = log.try_into_log::<CheckpointAckLogData>() else {
                return Ok(());
            };
            process_checkpoint_ack_log(state, &data, context)?;
        }

        _ => {
            // Some other log type, which we don't care about, skip it.
        }
    }

    Ok(())
}

fn process_deposit_intent_log<S: StateAccessor>(
    state: &mut S,
    data: &DepositIntentLogData,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Convert the account serial to account ID.
    let Some(dest_id) = state.find_account_id_by_serial(data.dest_acct_serial())? else {
        // Account serial not found, skip this deposit.
        //
        // TODO make this actually do something more sophisticated to make loss
        // of funds less likely
        return Ok(());
    };

    // Create the message payload containing the subject ID.
    // TODO make better handling for this like we have for ASM logs
    let mut msg_data = Vec::new();
    let subject_bytes: [u8; 32] = data.dest_subject().into();
    msg_data.extend_from_slice(&subject_bytes);

    let msg_payload = MsgPayload::new(data.amt().into(), msg_data);

    // Deliver the deposit message to the target account
    // TODO need to tweak this a bit to deal with the changes to epoch contexts
    account_processing::process_message(
        state,
        BRIDGE_GATEWAY_ACCT_ID,
        dest_id,
        msg_payload,
        context,
    )?;

    Ok(())
}

fn process_checkpoint_ack_log<S: StateAccessor>(
    state: &mut S,
    data: &CheckpointAckLogData,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Update the L1 view state with the acknowledged epoch.
    //
    // This records that a checkpoint has been observed on L1.
    state.l1_view_mut().set_asm_recorded_epoch(data.epoch());

    Ok(())
}
