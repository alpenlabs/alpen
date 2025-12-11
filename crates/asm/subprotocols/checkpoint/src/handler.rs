//! Checkpoint transaction handler.
//!
//! This module handles the processing of individual checkpoint transactions,
//! coordinating verification, state updates, and message forwarding.

use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{AsmLogEntry, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_logs::CheckpointUpdate;
use strata_asm_proto_checkpoint_txs::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinTxid};

use crate::{
    error::{CheckpointError, CheckpointResult},
    state::CheckpointState,
    verification::{
        construct_checkpoint_claim, validate_epoch_sequence, validate_l1_progression,
        validate_l2_progression, verify_checkpoint_proof, verify_checkpoint_signature,
    },
};

/// Process a checkpoint transaction.
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extract signed checkpoint from envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Verify signature
    if !verify_checkpoint_signature(&signed_checkpoint, &state.sequencer_cred) {
        return Err(CheckpointError::InvalidSignature);
    }

    let checkpoint = signed_checkpoint.payload();
    let batch_info = checkpoint.batch_info();

    // 3. Validate state transitions
    validate_epoch_sequence(state, batch_info.epoch())?;
    validate_l1_progression(state, batch_info.final_l1_block().height_u64())?;
    validate_l2_progression(state, batch_info.final_l2_block().slot())?;

    // 4. Get manifest hashes from auxiliary data (requested during pre-process phase)
    let prev_l1_height = state.last_checkpoint_l1.height_u64();
    let new_l1_height = batch_info.final_l1_block().height_u64();
    let manifest_hashes = verified_aux_data
        .get_manifest_hashes(prev_l1_height, new_l1_height)
        .map_err(|e| {
            logging::debug!(error = ?e, "Failed to retrieve manifest hashes");
            CheckpointError::MissingManifestHashes
        })?;

    // 5. Construct claim and verify proof (start values from state, end values from payload)
    let claim = construct_checkpoint_claim(state, checkpoint, &manifest_hashes)?;
    verify_checkpoint_proof(&state.checkpoint_predicate, &claim, checkpoint.proof())?;

    // 6. Update state with verified checkpoint
    state.update_with_checkpoint(checkpoint);

    // 7. Forward withdrawal intents to bridge
    forward_withdrawal_intents(checkpoint, relayer);

    // 8. Emit checkpoint update log
    emit_checkpoint_log(tx, checkpoint, relayer);

    Ok(())
}

/// Forward withdrawal intents to the bridge subprotocol.
///
/// Parses the OL logs from the checkpoint sidecar, filters for withdrawal intents
/// from the bridge gateway account, and forwards them to the bridge subprotocol.
fn forward_withdrawal_intents(checkpoint: &CheckpointPayload, relayer: &mut impl MsgRelayer) {
    let Some(logs) = checkpoint.sidecar().parse_ol_logs() else {
        logging::warn!(
            epoch = checkpoint.epoch(),
            "Failed to parse OL logs from checkpoint"
        );
        return;
    };

    let mut withdrawal_count = 0;

    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        let Some(withdrawal_data) =
            strata_codec::decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload()).ok()
        else {
            logging::debug!("Failed to decode withdrawal intent log payload");
            continue;
        };

        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            logging::debug!("Failed to parse withdrawal destination descriptor");
            continue;
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
        withdrawal_count += 1;
    }

    if withdrawal_count > 0 {
        logging::info!(
            withdrawal_count,
            epoch = checkpoint.epoch(),
            "Forwarded withdrawal intents to bridge"
        );
    }
}

/// Emit a checkpoint update log.
fn emit_checkpoint_log(
    tx: &TxInputRef<'_>,
    checkpoint: &CheckpointPayload,
    relayer: &mut impl MsgRelayer,
) {
    let checkpoint_txid = BitcoinTxid::new(&tx.tx().compute_txid());
    let checkpoint_update = CheckpointUpdate::from_payload(checkpoint, checkpoint_txid);

    match AsmLogEntry::from_log(&checkpoint_update) {
        Ok(log_entry) => {
            relayer.emit_log(log_entry);
            logging::info!(
                txid = %tx.tx().compute_txid(),
                epoch = checkpoint.epoch(),
                "Emitted checkpoint update log"
            );
        }
        Err(err) => {
            logging::error!(error = ?err, "Failed to encode checkpoint update log");
        }
    }
}
