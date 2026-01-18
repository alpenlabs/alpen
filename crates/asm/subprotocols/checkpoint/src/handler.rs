use bitcoin_bosd::Descriptor;
use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{ChainViewState, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_proto_checkpoint_txs::parser::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::OLLog;
use strata_codec::decode_buf_exact;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;

use crate::{
    errors::CheckpointValidationError, state::CheckpointState,
    verification::validate_checkpoint_payload,
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
    l1view: &ChainViewState,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) {
    let Ok(payload) = extract_signed_checkpoint_from_envelope(tx) else {
        logging::warn!("failed to extract checkpoint payload from envelope, ignoring");
        return;
    };
    let epoch = payload.inner().new_tip().epoch;

    logging::debug!(epoch, "processing checkpoint transaction");

    let current_l1_height = l1view.pow_state.last_verified_block.height_u32();
    match validate_checkpoint_payload(state, current_l1_height, &payload, verified_aux_data) {
        Ok(()) => {
            logging::info!(epoch, "checkpoint validated successfully");

            state.update_verified_tip(payload.inner().new_tip);

            forward_withdrawal_intents(relayer, payload.inner().sidecar().ol_logs());
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

/// Forwards withdrawal intents to the bridge subprotocol.
///
/// Filters OL logs from the bridge gateway account and attempts to decode them
/// as withdrawal intents. Successfully decoded withdrawal intents are relayed to
/// the bridge subprotocol. Logs that cannot be decoded are skipped as they may be
/// other log types from the same account.
///
/// # Panics
///
/// Panics if a withdrawal intent has a malformed destination descriptor, as this
/// indicates user funds have been destroyed on L2 but cannot be withdrawn on L1.
fn forward_withdrawal_intents(relayer: &mut impl MsgRelayer, logs: &[OLLog]) {
    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        // Attempt to decode as withdrawal intent log data
        // Logs from this account may have other formats, so skip if decoding fails
        let Ok(withdrawal_data) = decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        else {
            logging::debug!("Skipping log that is not a withdrawal intent");
            continue;
        };

        // Parse destination descriptor; panic on malformed descriptors
        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            // CRITICAL: User funds are destroyed on L2 but cannot be withdrawn on L1.
            logging::error!(
                "Failed to parse withdrawal destination descriptor - user funds may be lost"
            );
            panic!("malformed withdrawal destination descriptor");
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
    }
}
