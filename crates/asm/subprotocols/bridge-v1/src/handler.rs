use bitcoin::ScriptBuf;
use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::NewExportEntry;

use crate::{
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
///
/// # Arguments
/// * `state` - Mutable reference to the bridge state to be updated
/// * `parsed_tx` - The parsed transaction to handle
/// * `relayer` - The message relayer used for emitting logs
///
/// # Returns
/// * `Ok(())` if the transaction was processed successfully
/// * `Err(BridgeSubprotocolError)` if an error occurred during processing
pub(crate) fn handle_parsed_tx<'t>(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    relayer: &mut impl MsgRelayer,
    aux_data: &VerifiedAuxData,
    nn_script: &ScriptBuf,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(parsed_deposit_tx) => {
            let ParsedDepositTx { tx, info } = parsed_deposit_tx;
            state.process_deposit_tx(tx, &info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            state.process_withdrawal_fulfillment_tx(&info)?;
            Ok(())
        }
        ParsedTx::Commit(info) => {
            let prev_txout = aux_data.get_bitcoin_txout(&info.first_input_outpoint)?;
            if &prev_txout.script_pubkey != nn_script {
                return Err(BridgeSubprotocolError::InvalidSpentOutputLock);
            }

            if &info.second_output_script != nn_script {
                return Err(BridgeSubprotocolError::InvalidSpentOutputLock);
            }

            let unlock = state.process_commit_tx(&info)?;

            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log =
                NewExportEntry::new(container_id, unlock.to_export_entry());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:"));

            Ok(())
        }
    }
}

pub(crate) fn preprocess_parsed_tx<'t>(
    parsed_tx: ParsedTx<'t>,
    _state: &BridgeV1State,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(_) => {}
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Commit(info) => {
            collector.request_bitcoin_tx(info.first_input_outpoint.txid);
        }
    }
}
