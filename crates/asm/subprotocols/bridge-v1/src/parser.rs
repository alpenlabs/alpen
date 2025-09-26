use bitcoin::Transaction;
use strata_asm_common::TxInputRef;
use strata_asm_txs_bridge_v1::{
    constants::{DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    deposit::{DepositInfo, parse_deposit_tx},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, parse_withdrawal_fulfillment_tx},
};

use crate::BridgeSubprotocolError;

/// A parsed deposit transaction containing the raw transaction and extracted deposit information.
#[derive(Debug)]
pub struct ParsedDepositTx<'t> {
    pub tx: &'t Transaction,
    pub info: DepositInfo,
}

/// A parsed withdrawal fulfillment transaction containing the raw transaction and extracted
/// withdrawal information.
#[derive(Debug)]
pub struct ParsedWithdrawalFulfillmentTx<'t> {
    pub tx: &'t Transaction,
    pub info: WithdrawalFulfillmentInfo,
}

/// Represents a parsed transaction that can be either a deposit or withdrawal fulfillment.
#[derive(Debug)]
pub enum ParsedTx<'t> {
    /// A deposit transaction that locks Bitcoin funds in the bridge
    Deposit(ParsedDepositTx<'t>),
    /// A withdrawal fulfillment transaction that releases Bitcoin funds from the bridge
    WithdrawalFulfillment(ParsedWithdrawalFulfillmentTx<'t>),
}

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
pub fn parse_tx<'t>(tx: &'t TxInputRef<'t>) -> Result<ParsedTx<'t>, BridgeSubprotocolError> {
    match tx.tag().tx_type() {
        DEPOSIT_TX_TYPE => {
            let info = parse_deposit_tx(tx)?;
            let parsed_tx = ParsedDepositTx { tx: tx.tx(), info };
            Ok(ParsedTx::Deposit(parsed_tx))
        }
        WITHDRAWAL_TX_TYPE => {
            let info = parse_withdrawal_fulfillment_tx(tx)?;
            let parsed_tx = ParsedWithdrawalFulfillmentTx { tx: tx.tx(), info };
            Ok(ParsedTx::WithdrawalFulfillment(parsed_tx))
        }
        unsupported_type => Err(BridgeSubprotocolError::UnsupportedTxType(unsupported_type)),
    }
}
