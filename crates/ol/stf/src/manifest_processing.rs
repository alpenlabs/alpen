//! ASM manifest processing.

use strata_acct_types::{BitcoinAmount, L1BlockRecord, MsgPayload};
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
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_msg_types::{DEPOSIT_MSG_TYPE_ID, DepositMsgData};
use tracing::{debug, info, trace, warn};

use crate::{
    account_processing::{self, handle_misplaced_funds},
    constants::BRIDGE_GATEWAY_ACCT_ID,
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
};

/// Processes the manifests from a block, which is part of the epoch sealing
/// processing.
///
/// NOTE: Manifest processing is not expected to emit OL logs.
/// This also does NOT check the preseal root.
#[tracing::instrument(
    skip_all,
    fields(
        manifest_count = mf_cont.manifests().len(),
        epoch = state.cur_epoch(),
    ),
)]
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
        let real_height = next_manifest_height(orig_l1_height, i)?;
        if mf.height() != real_height {
            warn!(
                expected_height = real_height,
                got_height = mf.height(),
                index = i,
                "asm manifest height mismatch",
            );
            return Err(ExecError::AsmManifestHeightMismatch {
                expected: real_height,
                actual: mf.height(),
                index: i,
            });
        }
        trace!(
            height = real_height,
            log_count = mf.logs().len(),
            "processing asm manifest",
        );
        last = Some((real_height, mf));
        process_asm_manifest(state, real_height, mf, context)?;
    }

    if let Some((_last_height, _last_mf)) = last {
        // TODO this is where we would update the header, if we want to keep
        // that as defined in the spec
    }

    // 2. Finally, we can update the epoch to get it ready for the next epoch.
    let new_epoch = terminating_epoch
        .checked_add(1)
        .ok_or(ExecError::EpochOverflow)?;
    info!(
        from_epoch = terminating_epoch,
        to_epoch = new_epoch,
        last_l1_height = ?last.map(|(h, _)| h),
        "advancing epoch",
    );
    state.set_cur_epoch(new_epoch);

    Ok(())
}

fn next_manifest_height(last_l1_height: L1Height, index: usize) -> ExecResult<L1Height> {
    let offset = L1Height::try_from(index).map_err(|_| ExecError::AsmManifestHeightOverflow)?;
    last_l1_height
        .checked_add(offset)
        .and_then(|height| height.checked_add(1))
        .ok_or(ExecError::AsmManifestHeightOverflow)
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

    // 2. Accept the L1 block record into the ASM MMR.
    let rec = L1BlockRecord::new(*mf.blkid().as_ref(), *mf.wtxids_root().as_ref());
    state.append_l1_block_rec(real_height, rec);

    Ok(())
}

fn process_asm_log<S: IStateAccessorMut>(
    state: &mut S,
    log: &AsmLogEntry,
    real_height: L1Height,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Try to parse the log as an SPS-52 message.
    let Some(msg) = log.try_as_msg() else {
        // Not a valid message format, skip it.
        debug!(
            height = real_height,
            "skipping asm log: not an sps-52 message"
        );
        return Ok(());
    };

    // Match on the type ID to determine how to process the log.
    match msg.ty() {
        DEPOSIT_LOG_TYPE_ID => {
            let Ok(deposit) = log.try_into_log::<DepositLog>() else {
                debug!(
                    height = real_height,
                    "failed to decode deposit log; skipping"
                );
                return Ok(());
            };
            process_deposit_log(state, real_height, &deposit, context)?;
        }

        CHECKPOINT_TIP_UPDATE_LOG_TYPE => {
            // Parse the checkpoint tip update from the checkpoint subprotocol.
            let Ok(data) = log.try_into_log::<CheckpointTipUpdate>() else {
                debug!(
                    height = real_height,
                    "failed to decode checkpoint tip update log; skipping"
                );
                return Ok(());
            };
            process_checkpoint_tip_update(state, &data, context)?;
        }

        EE_PREDICATE_KEY_UPDATE_LOG_TYPE => {
            // Parse the per-snark-account predicate key update.
            let Ok(data) = log.try_into_log::<EePredicateKeyUpdate>() else {
                debug!(
                    height = real_height,
                    "failed to decode ee predicate key update log; skipping"
                );
                return Ok(());
            };
            process_ee_predicate_key_update(state, &data)?;
        }

        ty => {
            // Some other log type, which we don't care about, skip it.
            debug!(
                height = real_height,
                log_ty = ty,
                "ignoring unknown asm log type"
            );
        }
    }

    Ok(())
}

fn process_deposit_log<S: IStateAccessorMut>(
    state: &mut S,
    real_height: L1Height,
    deposit: &DepositLog,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let amt_btc = BitcoinAmount::from_sat(deposit.amount);

    // Parse the raw destination bytes into account serial + subject.
    let Ok(descriptor) = DepositDescriptor::decode_from_slice(&deposit.destination) else {
        // Malformed destination descriptor, sweep to limbo.
        let coin = Coin::new_unchecked(amt_btc);
        warn!(
            l1_height = real_height,
            amount_sat = deposit.amount,
            "limboing deposit with malformed destination descriptor",
        );
        handle_misplaced_funds(state, coin)?;
        return Ok(());
    };

    let acct_serial = *descriptor.dest_acct_serial();
    let subject_id = descriptor.dest_subject().to_subject_id();

    // Convert the account serial to account ID.
    let Some(dest_id) = state.find_account_id_by_serial(acct_serial)? else {
        // Account serial not found, sweep to limbo.
        let coin = Coin::new_unchecked(amt_btc);
        warn!(
            l1_height = real_height,
            ?acct_serial,
            amount_sat = deposit.amount,
            "limboing deposit for unknown account serial",
        );
        handle_misplaced_funds(state, coin)?;
        return Ok(());
    };

    // Create the message payload containing the typed deposit message.
    let deposit_msg = DepositMsgData::new(subject_id);
    let deposit_body = encode_to_vec(&deposit_msg)?;
    let deposit_data = OwnedMsg::new(DEPOSIT_MSG_TYPE_ID, deposit_body)
        .expect("deposit message body must fit into msg-fmt envelope")
        .to_vec();
    let msg_payload = MsgPayload::from_bytes(deposit.amount.into(), deposit_data)
        .expect("deposit message payload bytes must fit within SSZ max length");

    info!(
        l1_height = real_height,
        %dest_id,
        %acct_serial,
        ?subject_id,
        amount_sat = deposit.amount,
        "crediting deposit to account",
    );

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
    debug!(
        epoch = tip.epoch,
        l2_commitment = %tip.l2_commitment(),
        "asm recorded epoch updated",
    );
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_manifest_height_rejects_l1_height_overflow() {
        assert_eq!(next_manifest_height(0, 0).expect("height should fit"), 1);

        assert!(matches!(
            next_manifest_height(L1Height::MAX, 0),
            Err(ExecError::AsmManifestHeightOverflow)
        ));
        assert!(matches!(
            next_manifest_height(L1Height::MAX - 1, 1),
            Err(ExecError::AsmManifestHeightOverflow)
        ));
    }
}
