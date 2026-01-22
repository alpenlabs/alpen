use strata_asm_bridge_msgs::BridgeIncomingMsg;
use strata_asm_common::{MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_proto_checkpoint_txs::parser::extract_signed_checkpoint_from_envelope;
use strata_identifiers::L1Height;

use crate::{
    errors::CheckpointValidationError, state::CheckpointState,
    verification::validate_checkpoint_and_extract_withdrawal_intents,
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
    let Ok(payload) = extract_signed_checkpoint_from_envelope(tx) else {
        logging::warn!("failed to extract checkpoint payload from envelope, ignoring");
        return;
    };
    let epoch = payload.inner().new_tip().epoch;

    logging::debug!(epoch, "processing checkpoint transaction");

    match validate_checkpoint_and_extract_withdrawal_intents(state, current_l1_height, &payload, verified_aux_data) {
        Ok(withdrawal_intents) => {
            logging::info!(epoch, "checkpoint validated successfully");

            state.update_verified_tip(payload.inner().new_tip);

            for withdraw_output in withdrawal_intents {
                let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
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
            CheckpointValidationError::InvalidPayload(e) => {
                logging::warn!(epoch, error = %e, "invalid checkpoint payload");
            }
        },
    }
}
