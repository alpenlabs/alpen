//! ASM manifest processing.

use strata_acct_types::{
    ADMIN_MSG_ACCT_ID, AccountId, BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount, L1BlockRecord,
    MessageEntry, MsgPayload, MsgPayloadData,
};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::{
    CheckpointTipUpdate, DepositLog, EePredicateKeyUpdate, constants::AsmLogTypeId,
};
use strata_codec::encode_to_vec;
use strata_identifiers::{EpochCommitment, L1Height};
use strata_ledger_types::*;
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_msg_types::{DEPOSIT_MSG_TYPE_ID, DepositMsgData, PREDICATE_UPDATE_MSG_TYPE_ID};
use strata_predicate::PredicateKey;
use tracing::{debug, info, trace, warn};

use crate::{
    account_processing::{self, handle_misplaced_funds},
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
    msg_payload_coin::MsgPayloadCoin,
};

/// Buffers the ASM logs carried by a sequence of manifests into the intraepoch
/// state for later processing at the epoch terminal.
///
/// Manifests may be included in any block within an epoch; this does not imply
/// the block is an epoch terminal. The manifest heights must be strictly
/// sequential after the state's `last_l1_height`, which carries the running
/// cursor across blocks since `append_l1_block_rec` is called eagerly here. The
/// ASM-log *effects* are deferred to [`process_epoch_terminal`].
///
/// Accepts a plain slice rather than the per-block
/// [`OLAsmManifestContainer`](strata_ol_chain_types::OLAsmManifestContainer)
/// so callers replaying a whole epoch (e.g. checkpoint proving) are not bound
/// by the per-block `MAX_SEALING_MANIFEST_COUNT` limit.
///
/// NOTE: This does not apply any log effects, advance the epoch, or emit OL
/// logs.
#[tracing::instrument(
    skip_all,
    fields(
        manifest_count = manifests.len(),
        epoch = state.cur_epoch(),
    ),
)]
pub fn process_block_manifests<S: IStateAccessorMut>(
    state: &mut S,
    manifests: &[AsmManifest],
) -> ExecResult<()> {
    // The state's last seen height is the running cursor; new manifests are
    // strictly after it, regardless of which block in the epoch they arrive in.
    let orig_l1_height = state.last_l1_height();

    for (i, mf) in manifests.iter().enumerate() {
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
            "buffering asm manifest logs",
        );
        handle_asm_manifest(state, real_height, mf)?;
    }

    Ok(())
}

/// Processes the epoch terminal: drains all buffered ASM logs (applying their
/// effects), resets the intraepoch state, and advances the epoch.
///
/// This is invoked at the block flagged as the epoch terminal.
#[tracing::instrument(
    skip_all,
    fields(
        pending_logs = state.pending_asm_logs_len(),
        epoch = state.cur_epoch(),
    ),
)]
pub fn process_epoch_terminal<S: IStateAccessorMut>(
    state: &mut S,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let terminating_epoch = state.cur_epoch();

    // 1. Snapshot the buffered ASM logs into a local list so we can apply their
    // effects and then reset the buffer without index/borrow hazards. The
    // effect handlers do not push new pending logs, so the snapshot is stable.
    let pending: Vec<PendingAsmLog> = (0..state.pending_asm_logs_len())
        .map(|idx| {
            state
                .get_pending_asm_log(idx)
                .expect("pending asm log index within bounds")
        })
        .collect();

    // 2. Apply the effects of each buffered log in order.
    for entry in &pending {
        process_asm_log(state, entry.log(), entry.height(), context)?;
    }

    // 3. Reset the now-consumed intraepoch buffer.
    state.reset_intraepoch_state();

    // 4. Advance the epoch to get it ready for the next epoch.
    let new_epoch = terminating_epoch
        .checked_add(1)
        .ok_or(ExecError::EpochOverflow)?;
    info!(
        from_epoch = terminating_epoch,
        to_epoch = new_epoch,
        drained_logs = pending.len(),
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

/// Handles a single manifest.
///
/// Appends each of its logs to the intraepoch pending-log buffer and eagerly
/// accepts the L1 block record for the manifest into the ASM MMR (which
/// advances `last_l1_height`).
fn handle_asm_manifest<S: IStateAccessorMut>(
    state: &mut S,
    real_height: L1Height,
    mf: &AsmManifest,
) -> ExecResult<()> {
    // 1. Buffer each of the logs for processing at the epoch terminal.
    for log in mf.logs() {
        state.try_append_pending_asm_log(PendingAsmLog::new(real_height, log.clone()))?;
    }

    // 2. Accept the L1 block record into the ASM MMR. This stays eager so that
    // `last_l1_height` tracks the running cursor across blocks in the epoch.
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
    match AsmLogTypeId::try_from(msg.ty()) {
        Ok(AsmLogTypeId::Deposit) => {
            let Ok(deposit) = log.try_into_log::<DepositLog>() else {
                debug!(
                    height = real_height,
                    "failed to decode deposit log; skipping"
                );
                return Ok(());
            };
            process_deposit_log(state, real_height, &deposit, context)?;
        }

        Ok(AsmLogTypeId::CheckpointTipUpdate) => {
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

        Ok(AsmLogTypeId::EePredicateKeyUpdate) => {
            // Parse the per-snark-account predicate key update.
            let Ok(data) = log.try_into_log::<EePredicateKeyUpdate>() else {
                debug!(
                    height = real_height,
                    "failed to decode ee predicate key update log; skipping"
                );
                return Ok(());
            };
            process_ee_predicate_key_update(state, &data, context)?;
        }

        Ok(ty) => {
            // Some other log type, which we don't care about, skip it.
            debug!(height = real_height, ?ty, "ignoring unknown asm log type");
        }

        Err(_) => {
            debug!(
                height = real_height,
                log_ty = msg.ty(),
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

    // Resolve the destination before minting any value.  All the fallible steps
    // run here so their errors propagate cleanly; only once we know whether the
    // deposit is credited or swept do we mint the coin.  `None` means sweep to
    // limbo.
    let resolved = resolve_deposit_destination(state, real_height, deposit)?;

    // Mint the coin exactly once, now that the deposit has been fully validated,
    // and hand it to whichever sink takes it (both consume it on all paths).
    let coin = Coin::new_unchecked(amt_btc);
    match resolved {
        None => {
            handle_misplaced_funds(state, coin)?;
        }
        Some((dest_id, deposit_data)) => {
            let msg_payload = MsgPayloadCoin::new(coin, deposit_data);

            // Deliver the deposit message to the target account.
            // TODO(STR-3677): need to tweak this a bit to deal with the changes to epoch contexts
            account_processing::process_message(
                state,
                BRIDGE_GATEWAY_ACCT_ID,
                dest_id,
                msg_payload,
                context,
            )?;
        }
    }

    Ok(())
}

/// Resolves a deposit's destination account and message payload without touching
/// any value.
///
/// Returns [`None`] when the deposit should be swept to limbo (malformed
/// descriptor or unknown account serial), or `Some` with the target account and
/// its deposit message data.  Keeping this coin-free lets its fallible steps
/// propagate errors without a live [`Coin`] in scope.
fn resolve_deposit_destination<S: IStateAccessorMut>(
    state: &S,
    real_height: L1Height,
    deposit: &DepositLog,
) -> ExecResult<Option<(AccountId, MsgPayloadData)>> {
    // Parse the raw destination bytes into account serial + subject.
    let Ok(descriptor) = DepositDescriptor::decode_from_slice(&deposit.destination) else {
        // Malformed destination descriptor, sweep to limbo.
        warn!(
            l1_height = real_height,
            amount_sat = deposit.amount,
            "limboing deposit with malformed destination descriptor",
        );
        return Ok(None);
    };

    let acct_serial = *descriptor.dest_acct_serial();
    let subject_id = descriptor.dest_subject().to_subject_id();

    // Convert the account serial to account ID.
    let Some(dest_id) = state.find_account_id_by_serial(acct_serial)? else {
        // Account serial not found, sweep to limbo.
        warn!(
            l1_height = real_height,
            ?acct_serial,
            amount_sat = deposit.amount,
            "limboing deposit for unknown account serial",
        );
        return Ok(None);
    };

    // Create the message payload containing the typed deposit message.
    let deposit_msg = DepositMsgData::new(subject_id);
    let deposit_body = encode_to_vec(&deposit_msg)?;
    let deposit_data = OwnedMsg::new(DEPOSIT_MSG_TYPE_ID, deposit_body)
        .expect("deposit message body must fit into msg-fmt envelope")
        .to_vec();
    let deposit_data: MsgPayloadData = deposit_data
        .try_into()
        .expect("deposit message payload bytes must fit within SSZ max length");

    info!(
        l1_height = real_height,
        %dest_id,
        %acct_serial,
        ?subject_id,
        amount_sat = deposit.amount,
        "crediting deposit to account",
    );

    Ok(Some((dest_id, deposit_data)))
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
    context: &BasicExecContext<'_>,
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
    let update_msg = build_predicate_update_message(&new_vk, context.epoch());
    let applied = state.update_account(acct_id, |astate| -> StateResult<bool> {
        // Skip if the target is not a snark account; non-snark accounts have
        // no predicate key to update.
        if let Ok(snark) = astate.as_snark_account_mut() {
            // The rotation is not applied here. It lands in the account's
            // inbox and takes effect when an account update consumes the
            // message (see `process_update_tx`). That makes the update that
            // consumes the message the last one verified under the old key —
            // the consensus-level fork boundary of the Alpen upgrade design —
            // and gives the EE a deterministic position in its inbox
            // ordering to derive the fork activation from. Applying the key
            // immediately would instead reject every in-flight update still
            // proven under the old key.
            snark.insert_inbox_message(update_msg)?;
            Ok(true)
        } else {
            Ok(false)
        }
    })??;

    if !applied {
        warn!(
            %acct_serial,
            %acct_id,
            "dropping ee predicate key update for non-snark account"
        );
    }

    Ok(())
}

/// Builds the inbox message announcing a predicate key rotation.
///
/// The message carries no value; its body is the SSZ encoding of the new
/// predicate key, wrapped in the standard SPS-52 message format under
/// [`PREDICATE_UPDATE_MSG_TYPE_ID`]. The source is the admin message account
/// id, a reserved system id that no ledger account can occupy.
fn build_predicate_update_message(new_vk: &PredicateKey, cur_epoch: u32) -> MessageEntry {
    let body = ssz::Encode::as_ssz_bytes(new_vk);
    let msg = OwnedMsg::new(PREDICATE_UPDATE_MSG_TYPE_ID, body)
        .expect("predicate update message type id is in bounds");
    let payload = MsgPayload::from_bytes_valueless(msg.to_vec())
        .expect("predicate key fits in message payload");
    MessageEntry::new(ADMIN_MSG_ACCT_ID, cur_epoch, payload)
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
