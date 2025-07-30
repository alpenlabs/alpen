use strata_asm_common::{AsmLogEntry, MsgRelayer};
use strata_asm_logs::NewExportEntry;

use crate::{
    errors::BridgeSubprotocolError,
    state::BridgeV1State,
    txs::{ParsedDepositTx, ParsedTx, ParsedWithdrawalFulfillmentTx},
};

pub(crate) fn handle_parsed_tx<'t>(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(parsed_deposit_tx) => {
            let ParsedDepositTx { tx, info } = parsed_deposit_tx;
            state.process_deposit_tx(tx, &info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(parsed_withdrawal_fulfillment) => {
            let ParsedWithdrawalFulfillmentTx { tx, info } = parsed_withdrawal_fulfillment;
            let unlock = state.process_withdrawal_fulfillment_tx(tx, &info)?;

            // FIXME: This is a placeholder for the actual container ID logic.
            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log =
                NewExportEntry::new(container_id, unlock.to_export_entry());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:"));

            Ok(())
        }
    }
}
