//! Checkpoint transaction handler.
//!
//! This module handles the processing of individual checkpoint transactions,
//! coordinating verification, state updates, and message forwarding.

use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{AsmLogEntry, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_logs::CheckpointUpdateSsz;
use strata_asm_proto_checkpoint_txs::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::{BatchInfo, CheckpointClaim, CheckpointPayload};
use strata_codec::decode_buf_exact;
use strata_crypto::schnorr::verify_schnorr_sig;
use strata_identifiers::CredRule;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinTxid};

use crate::{
    error::{CheckpointError, CheckpointResult},
    state::CheckpointState,
    utils::{compute_manifest_hashes_commitment, get_manifest_hashes},
};

/// Process a checkpoint transaction.
///
/// Steps:
/// 1. Extract signed checkpoint from envelope
/// 2. Verify signature
/// 3. Validate state transitions (epoch, L1/L2 progression)
/// 4. Validate start values match expected state
/// 5. Get manifest hashes from auxiliary data
/// 6. Construct claim and verify proof
/// 7. Update state with verified checkpoint
/// 8. Forward withdrawal intents to bridge
/// 9. Emit checkpoint update log
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extract signed checkpoint from envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Verify signature
    let payload_hash = signed_checkpoint.inner.compute_hash();
    if !verify_signature(
        &payload_hash,
        &signed_checkpoint.signature,
        state.sequencer_cred(),
    ) {
        return Err(CheckpointError::InvalidSignature);
    }

    let checkpoint_payload = &signed_checkpoint.inner;
    let batch_info = &checkpoint_payload.commitment.batch_info;

    // 3. Validate start values match expected state
    validate_start_values(state, batch_info)?;

    // 4. Validate state transitions (epoch, L1/L2 progression)
    validate_state_transitions(state, batch_info)?;

    // 5. Get manifest hashes from auxiliary data
    let manifest_hashes = get_manifest_hashes(state, batch_info, verified_aux_data)?;

    // 6. Construct claim and verify proof
    let pre_state_root = state.pre_state_root();
    let input_msgs_commitment = compute_manifest_hashes_commitment(&manifest_hashes);
    let claim =
        CheckpointClaim::from_payload(checkpoint_payload, pre_state_root, input_msgs_commitment);

    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.to_bytes(), &checkpoint_payload.proof)?;

    // 7. Update state with verified checkpoint
    state.update_with_checkpoint(checkpoint_payload);

    // 8. Forward withdrawal intents to bridge
    forward_withdrawal_intents(checkpoint_payload, relayer);

    // 9. Emit checkpoint update log
    let checkpoint_txid = BitcoinTxid::from(tx.tx().compute_txid());
    emit_checkpoint_log(checkpoint_payload, checkpoint_txid, relayer)?;

    Ok(())
}

/// Verify the checkpoint signature based on the credential rule.
fn verify_signature(
    payload_hash: &strata_identifiers::Buf32,
    signature: &strata_identifiers::Buf64,
    cred_rule: &CredRule,
) -> bool {
    match cred_rule {
        CredRule::SchnorrKey(pk) => verify_schnorr_sig(signature, payload_hash, pk),
        _ => false,
    }
}

/// Validate that checkpoint start values match expected state.
fn validate_start_values(state: &CheckpointState, batch_info: &BatchInfo) -> CheckpointResult<()> {
    // L1 range start must match last covered L1
    let last_l1_height = state.last_covered_l1_height();
    let l1_start = batch_info.l1_range.start.height;
    if l1_start != last_l1_height {
        return Err(CheckpointError::InvalidL1Progression {
            previous: last_l1_height,
            new: l1_start,
        });
    }

    // L2 range start must match last terminal slot (if any)
    if let Some(last_l2_slot) = state.last_l2_terminal_slot() {
        let l2_start = batch_info.l2_range.start.slot();
        if l2_start != last_l2_slot {
            return Err(CheckpointError::InvalidL2Progression {
                previous: last_l2_slot,
                new: l2_start,
            });
        }
    }

    Ok(())
}

/// Validate state transitions: epoch sequence, L1 height progression, L2 slot progression.
fn validate_state_transitions(
    state: &CheckpointState,
    batch_info: &BatchInfo,
) -> CheckpointResult<()> {
    // Epoch must be sequential
    let expected_epoch = state.expected_next_epoch();
    if batch_info.epoch != expected_epoch {
        return Err(CheckpointError::InvalidEpoch {
            expected: expected_epoch,
            actual: batch_info.epoch,
        });
    }

    // L1 end height must progress beyond last covered L1
    let last_l1_height = state.last_covered_l1_height();
    let l1_end = batch_info.l1_range.end.height;
    if l1_end <= last_l1_height {
        return Err(CheckpointError::InvalidL1Progression {
            previous: last_l1_height,
            new: l1_end,
        });
    }

    // L2 end slot must progress beyond last terminal (if any)
    if let Some(last_l2_slot) = state.last_l2_terminal_slot() {
        let l2_end = batch_info.l2_range.end.slot();
        if l2_end <= last_l2_slot {
            return Err(CheckpointError::InvalidL2Progression {
                previous: last_l2_slot,
                new: l2_end,
            });
        }
    }

    Ok(())
}

/// Forward withdrawal intents to the bridge subprotocol.
///
/// Parses the OL logs from the checkpoint sidecar, filters for withdrawal intents
/// from the bridge gateway account, and forwards them to the bridge subprotocol.
fn forward_withdrawal_intents(checkpoint: &CheckpointPayload, relayer: &mut impl MsgRelayer) {
    let Some(logs) = checkpoint.sidecar.parse_ol_logs() else {
        logging::warn!(
            epoch = checkpoint.epoch(),
            "Failed to parse OL logs from checkpoint"
        );
        return;
    };

    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        // TODO: Clarify expected behavior for malformed logs (currently skipped silently)
        let Ok(withdrawal_data) = decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        else {
            logging::warn!("Failed to decode withdrawal intent log payload");
            continue;
        };

        // TODO: Clarify expected behavior for malformed descriptors (currently skipped silently)
        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            logging::warn!("Failed to parse withdrawal destination descriptor");
            continue;
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
    }
}

/// Emit a checkpoint update log entry.
fn emit_checkpoint_log(
    checkpoint: &CheckpointPayload,
    checkpoint_txid: BitcoinTxid,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    let checkpoint_update = CheckpointUpdateSsz::from_payload(checkpoint, checkpoint_txid);
    let log_entry = AsmLogEntry::from_log(&checkpoint_update)?;
    relayer.emit_log(log_entry);
    Ok(())
}
