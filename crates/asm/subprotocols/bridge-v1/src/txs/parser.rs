use strata_asm_common::TxInputRef;

use crate::{
    constants::{DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    errors::BridgeSubprotocolError,
    txs::{
        ParsedDepositTx, ParsedTx, ParsedWithdrawalFulfillmentTx, deposit::extract_deposit_info,
        withdrawal_fulfillment::extract_withdrawal_info,
    },
};

/// Parses a transaction into a structured format based on its type.
///
/// This function examines the transaction type from the tag and extracts relevant
/// information for either deposit or withdrawal fulfillment transactions.
///
/// # Arguments
///
/// * `tx` - The transaction input reference to parse
///
/// # Returns
///
/// * `Ok(ParsedTx::Deposit)` - For deposit transactions with extracted deposit information
/// * `Ok(ParsedTx::WithdrawalFulfillment)` - For withdrawal transactions with extracted withdrawal
///   information
/// * `Err(BridgeSubprotocolError::UnsupportedTxType)` - For unsupported transaction types
///
/// # Errors
///
/// Returns an error if:
/// - The transaction type is not supported by the bridge subprotocol
/// - The transaction data extraction fails (malformed transaction structure)
pub(crate) fn parse_tx<'t>(tx: &'t TxInputRef<'t>) -> Result<ParsedTx<'t>, BridgeSubprotocolError> {
    match tx.tag().tx_type() {
        DEPOSIT_TX_TYPE => {
            let info = extract_deposit_info(tx)?;
            let parsed_tx = ParsedDepositTx { tx: tx.tx(), info };
            Ok(ParsedTx::Deposit(parsed_tx))
        }
        WITHDRAWAL_TX_TYPE => {
            let info = extract_withdrawal_info(tx)?;
            let parsed_tx = ParsedWithdrawalFulfillmentTx { tx: tx.tx(), info };
            Ok(ParsedTx::WithdrawalFulfillment(parsed_tx))
        }
        unsupported_type => Err(BridgeSubprotocolError::UnsupportedTxType(unsupported_type)),
    }
}
