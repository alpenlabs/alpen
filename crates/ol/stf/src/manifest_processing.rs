//! ASM manifest processing.

use strata_acct_types::MsgPayload;
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::{
    CheckpointTipUpdate, DepositLog, EePredicateKeyUpdate,
    constants::{
        CHECKPOINT_TIP_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID, EE_PREDICATE_KEY_UPDATE_LOG_TYPE,
    },
};
use strata_codec::encode_to_vec;
use strata_identifiers::{EpochCommitment, L1Height};
use strata_ledger_types::*;
use strata_msg_fmt::Msg;
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_msg_types::DepositMsgData;
use tracing::warn;

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
pub fn process_block_manifests<S: IStateAccessorMut>(
    state: &mut S,
    mf_cont: &OLL1ManifestContainer,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let terminating_epoch = state.cur_epoch();

    // 1. Process all the manifests.
    let orig_l1_height = state.last_l1_height();
    let mut last = None;

    for (i, mf) in mf_cont.manifests().iter().enumerate() {
        // New manifests in a segment are strictly after the state's current
        // last seen height.
        let real_height = orig_l1_height + i as u32 + 1;
        if mf.height() != real_height {
            return Err(ExecError::ChainIntegrity);
        }
        last = Some((real_height, mf));
        process_asm_manifest(state, real_height, mf, context)?;
    }

    if let Some((_last_height, _last_mf)) = last {
        // TODO this is where we would update the header, if we want to keep
        // that as defined in the spec
    }

    // 2. Finally, we can update the epoch to get it ready for the next epoch.
    state.set_cur_epoch(terminating_epoch + 1);

    Ok(())
}

fn process_asm_manifest<S: IStateAccessorMut>(
    state: &mut S,
    real_height: L1Height,
    mf: &AsmManifest,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // 1. Process each of the logs.
    for log in mf.logs() {
        process_asm_log(state, log, real_height, context)?;
    }

    // 2. Accept the manifest into the ASM MMR.
    state.append_manifest(real_height, mf.clone());

    Ok(())
}

fn process_asm_log<S: IStateAccessorMut>(
    state: &mut S,
    log: &AsmLogEntry,
    _real_height: L1Height,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Try to parse the log as an SPS-52 message.
    let Some(msg) = log.try_as_msg() else {
        // Not a valid message format, skip it.
        return Ok(());
    };

    // Match on the type ID to determine how to process the log.
    match msg.ty() {
        DEPOSIT_LOG_TYPE_ID => {
            let Ok(deposit) = log.try_into_log::<DepositLog>() else {
                return Ok(());
            };
            process_deposit_log(state, &deposit, context)?;
        }

        CHECKPOINT_TIP_UPDATE_LOG_TYPE => {
            // Parse the checkpoint tip update from the checkpoint subprotocol.
            let Ok(data) = log.try_into_log::<CheckpointTipUpdate>() else {
                return Ok(());
            };
            process_checkpoint_tip_update(state, &data, context)?;
        }

        EE_PREDICATE_KEY_UPDATE_LOG_TYPE => {
            // Parse the per-snark-account predicate key update.
            let Ok(data) = log.try_into_log::<EePredicateKeyUpdate>() else {
                return Ok(());
            };
            process_ee_predicate_key_update(state, &data)?;
        }

        _ => {
            // Some other log type, which we don't care about, skip it.
        }
    }

    Ok(())
}

fn process_deposit_log<S: IStateAccessorMut>(
    state: &mut S,
    deposit: &DepositLog,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Parse the raw destination bytes into account serial + subject.
    let Ok(descriptor) = DepositDescriptor::decode_from_slice(&deposit.destination) else {
        // Malformed destination descriptor, skip this deposit.
        warn!(amount = %deposit.amount, "dropping deposit with malformed destination descriptor");
        return Ok(());
    };

    let acct_serial = *descriptor.dest_acct_serial();
    let subject_id = descriptor.dest_subject().to_subject_id();

    // Convert the account serial to account ID.
    let Some(dest_id) = state.find_account_id_by_serial(acct_serial)? else {
        // Account serial not found, skip this deposit.
        //
        // TODO make this actually do something more sophisticated to make loss
        // of funds less likely
        warn!(?acct_serial, amount = %deposit.amount, "dropping deposit for unknown account serial");
        return Ok(());
    };

    // Create the message payload containing the deposit message data.
    let deposit_msg = DepositMsgData::new(subject_id);
    let deposit_data = encode_to_vec(&deposit_msg)?;
    let msg_payload = MsgPayload::new(deposit.amount.into(), deposit_data);

    // Deliver the deposit message to the target account.
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

fn process_checkpoint_tip_update<S: IStateAccessorMut>(
    state: &mut S,
    data: &CheckpointTipUpdate,
    _context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let tip = data.tip();
    let epoch_commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());
    state.set_asm_recorded_epoch(epoch_commitment);

    Ok(())
}

fn process_ee_predicate_key_update<S: IStateAccessorMut>(
    state: &mut S,
    data: &EePredicateKeyUpdate,
) -> ExecResult<()> {
    let acct_serial = data.account();

    // Resolve the account serial. Skip if not found, matching the deposit
    // handler convention. ASM manifests cannot be rejected without halting
    // checkpoint progress, so we log and continue.
    let Some(acct_id) = state.find_account_id_by_serial(acct_serial)? else {
        warn!(
            ?acct_serial,
            "dropping ee predicate key update for unknown account serial"
        );
        return Ok(());
    };

    let new_vk = data.new_predicate().clone();
    let applied = state.update_account(acct_id, |astate| {
        // Skip if the target is not a snark account; non-snark accounts have
        // no predicate key to update.
        if let Ok(snark) = astate.as_snark_account_mut() {
            snark.set_update_vk(new_vk);
            true
        } else {
            false
        }
    })?;

    if !applied {
        warn!(
            %acct_serial,
            %acct_id,
            "dropping ee predicate key update for non-snark account"
        );
    }

    Ok(())
}
