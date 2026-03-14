use strata_asm_bridge_msgs::BridgeIncomingMsg;
use strata_asm_common::{AsmLogEntry, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_logs::CheckpointTipUpdate;
use strata_asm_txs_checkpoint::extract_checkpoint_from_envelope;
use strata_identifiers::L1Height;

use crate::{
    errors::CheckpointValidationError,
    state::CheckpointState,
    verification::{
        ValidatedCheckpointWithdrawals, validate_checkpoint_and_extract_withdrawal_intents,
    },
};

/// Processes a checkpoint transaction from L1.
///
/// Extracts and validates the checkpoint payload from the transaction envelope.
/// If the payload cannot be extracted or validation fails, the transaction is
/// ignored and logged. On successful validation, updates the verified tip and
/// forwards any withdrawal intents to the bridge subprotocol.
///
/// # Panics
///
/// Panics if the required auxiliary data (ASM manifest hashes) is not provided or withdrawal intent
/// has a malformed descriptor.
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    current_l1_height: L1Height,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) {
    let Ok(envelope) = extract_checkpoint_from_envelope(tx) else {
        logging::warn!("failed to extract checkpoint payload from envelope, ignoring");
        return;
    };
    let epoch = envelope.payload.new_tip().epoch;

    logging::debug!(epoch, "processing checkpoint transaction");

    match validate_checkpoint_and_extract_withdrawal_intents(
        state,
        current_l1_height,
        &envelope,
        verified_aux_data,
    ) {
        Ok(ValidatedCheckpointWithdrawals {
            withdrawal_intents,
            verified_withdrawals,
        }) => {
            logging::info!(epoch, "checkpoint validated successfully");

            state.deduct_withdrawals(verified_withdrawals);

            let new_tip = envelope.payload.new_tip;
            state.update_verified_tip(new_tip);

            let checkpoint_tip_update = CheckpointTipUpdate::new(new_tip);
            let log_entry = AsmLogEntry::from_log(&checkpoint_tip_update)
                .expect("CheckpointTipUpdate encoding is infallible for fixed-size SSZ");
            relayer.emit_log(log_entry);

            for (output, selected_operator) in withdrawal_intents {
                let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal {
                    output,
                    selected_operator,
                };
                relayer.relay_msg(&bridge_msg);
            }
        }
        Err(e) => match e {
            CheckpointValidationError::InvalidAux(e) => {
                // CRITICAL: We must panic here rather than ignore the error.
                //
                // The checkpoint payload itself specifies which L1 heights it covers, and we verify
                // that:
                // 1. The L1 range doesn't go backwards
                // 2. The L1 range doesn't exceed the current L1 tip
                //
                // Since we only request auxiliary data that MUST be valid and available,
                // invalid aux data indicates aux data was not provided. If we silently ignored this
                // error instead of panicking, valid checkpoints could be ignored as
                // being invalid.
                logging::error!(epoch, error = %e, "invalid aux data");
                panic!("invalid aux");
            }
            CheckpointValidationError::InvalidSequencerPredicate(e) => {
                logging::warn!(epoch, error = %e, "sequencer predicate verification failed");
            }
            CheckpointValidationError::InvalidPayload(e) => {
                logging::warn!(epoch, error = %e, "invalid checkpoint payload");
            }
        },
    }
}
