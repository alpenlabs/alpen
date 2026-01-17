use bitcoin_bosd::Descriptor;
use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
use strata_asm_common::{ChainViewState, MsgRelayer, TxInputRef, VerifiedAuxData, logging};
use strata_asm_proto_checkpoint_txs::parser::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::OLLog;
use strata_codec::decode_buf_exact;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;

use crate::{
    errors::CheckpointResult, state::CheckpointState, verification::validate_checkpoint_payload,
};

/// Processes a checkpoint transaction from L1.
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    l1view: &ChainViewState,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extracts the signed checkpoint from the transaction envelope
    let payload = extract_signed_checkpoint_from_envelope(tx)?;

    // 2. Validates the checkpoint payload (signature, epoch progression, proof)
    let current_l1_height = l1view.pow_state.last_verified_block.height_u32();
    validate_checkpoint_payload(state, current_l1_height, &payload, verified_aux_data)?;

    // 3. Updates the state with the new verified checkpoint tip
    state.update_verified_tip(payload.inner().new_tip);

    // 4. Forwards withdrawal intents to the bridge subprotocol
    forward_withdrawal_intents(relayer, payload.inner().sidecar().ol_logs());

    Ok(())
}

/// Forwards withdrawal intents to the bridge subprotocol.
///
/// Filters OL logs for withdrawal intents from the bridge gateway account,
/// decodes the withdrawal data, and relays them to the bridge subprotocol.
/// Malformed logs are skipped with a warning.
fn forward_withdrawal_intents(relayer: &mut impl MsgRelayer, logs: &[OLLog]) {
    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        // Decode withdrawal intent log data; skip malformed logs
        let Ok(withdrawal_data) = decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        else {
            logging::warn!("Failed to decode withdrawal intent log payload");
            continue;
        };

        // Parse destination descriptor; skip malformed descriptors
        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            logging::warn!("Failed to parse withdrawal destination descriptor");
            continue;
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        let bridge_msg = BridgeIncomingMsg::DispatchWithdrawal(withdraw_output);
        relayer.relay_msg(&bridge_msg);
    }
}
