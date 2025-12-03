use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::NewExportEntry;

use crate::{
    SlashValidationError,
    errors::BridgeSubprotocolError,
    parser::{ParsedDepositTx, ParsedTx},
    state::BridgeV1State,
};

/// Handles parsed transactions and update the bridge state accordingly.
///
/// # Transaction Types and Log Behavior:
/// - **Deposit**: Processes the deposit transaction without emitting logs
/// - **WithdrawalFulfillment**: Processes the withdrawal and emits a withdrawal processed log via
///   the relayer to notify other components of the processed withdrawal
/// - **Slash**: Validates the stake connector is locked to any valid N/N multisig (current or
///   historical) and processes slashing
///
/// # Arguments
/// * `state` - Mutable reference to the bridge state to be updated
/// * `parsed_tx` - The parsed transaction to handle
/// * `verified_aux_data` - Auxiliary data containing referenced transaction outputs
/// * `relayer` - The message relayer used for emitting logs
///
/// # Returns
/// * `Ok(())` if the transaction was processed successfully
/// * `Err(BridgeSubprotocolError)` if an error occurred during processing
pub(crate) fn handle_parsed_tx<'t>(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(parsed_deposit_tx) => {
            let ParsedDepositTx { tx, info } = parsed_deposit_tx;
            state.process_deposit_tx(tx, &info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            let unlock = state.process_withdrawal_fulfillment_tx(&info)?;

            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log = NewExportEntry::new(container_id, unlock.compute_hash());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:PG"));

            Ok(())
        }
        ParsedTx::Slash(info) => {
            // Validate that the stake connector (second input) is locked to a valid N/N
            // multisig script from any historical configuration.
            //
            // This ensures this is a valid slash transaction that was presigned by an N/N
            // multisig set and not some arbitrary transaction posted by someone else.
            //
            // We check against all historical scripts (which includes the current one)
            // because when multiple operators are slashed in sequence, the second slash
            // will reference a stake connector from the operator set configuration before
            // the first slash occurred.
            let stake_connector_script = &verified_aux_data
                .get_bitcoin_txout(info.second_inpoint().outpoint())?
                .script_pubkey;

            // Validate against any historical N/N script (includes current configuration)
            if !state
                .operators()
                .historical_nn_scripts()
                .any(|script| script == stake_connector_script)
            {
                return Err(SlashValidationError::InvalidStakeConnectorScript.into());
            };

            // Remove the slashed operator
            state.remove_operator(info.header_aux().operator_idx());

            Ok(())
        }
    }
}

/// Pre-processes a parsed transaction to collect auxiliary data requests.
///
/// This function inspects the transaction type and requests any additional data needed
/// for full verification during the main processing phase. Currently handles:
///
/// - **Deposit transactions**: No auxiliary data required
/// - **Withdrawal fulfillment transactions**: No auxiliary data required
/// - **Slash transactions**: Requests the conflicting Bitcoin transaction referenced in the slash
///   proof to enable verification of operator double-signing
///
/// # Parameters
///
/// - `parsed_tx` - The parsed transaction to pre-process
/// - `_state` - Current bridge state (unused, reserved for future use)
/// - `collector` - Collector for accumulating auxiliary data requests
pub(crate) fn preprocess_parsed_tx<'t>(
    parsed_tx: ParsedTx<'t>,
    _state: &BridgeV1State,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(_) => {}
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Slash(info) => {
            collector.request_bitcoin_tx(info.second_inpoint().0.txid);
        }
    }
}
