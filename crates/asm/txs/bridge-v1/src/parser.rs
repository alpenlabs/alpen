use std::convert::TryInto;

use bitcoin::Transaction;
use strata_asm_common::TxInputRef;

use crate::{
    constants::BridgeTxType,
    deposit::{DepositInfo, parse_deposit_tx},
    errors::BridgeTxParseError,
    slash::{SlashInfo, parse_slash_tx},
    unstake::{UnstakeInfo, parse_unstake_tx},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, parse_withdrawal_fulfillment_tx},
};

/// A parsed deposit transaction containing the raw transaction and extracted deposit information.
#[derive(Debug)]
pub struct ParsedDepositTx<'t> {
    pub tx: &'t Transaction,
    pub info: DepositInfo,
}

/// Represents a parsed transaction that can be either a deposit or withdrawal fulfillment.
#[derive(Debug)]
pub enum ParsedTx<'t> {
    /// A deposit transaction that locks Bitcoin funds in the bridge
    Deposit(ParsedDepositTx<'t>),
    /// A withdrawal fulfillment transaction that releases Bitcoin funds from the bridge
    WithdrawalFulfillment(WithdrawalFulfillmentInfo),
    /// A slash transaction that penalizes a misbehaving operator
    Slash(SlashInfo),
    /// An unstake transaction to exit from the bridge
    Unstake(UnstakeInfo),
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
pub fn parse_tx<'t>(tx: &'t TxInputRef<'t>) -> Result<ParsedTx<'t>, BridgeTxParseError> {
    match tx.tag().tx_type().try_into() {
        Ok(BridgeTxType::Deposit) => {
            let info = parse_deposit_tx(tx)?;
            let parsed_tx = ParsedDepositTx { tx: tx.tx(), info };
            Ok(ParsedTx::Deposit(parsed_tx))
        }
        Ok(BridgeTxType::WithdrawalFulfillment) => {
            let info = parse_withdrawal_fulfillment_tx(tx)?;
            Ok(ParsedTx::WithdrawalFulfillment(info))
        }
        Ok(BridgeTxType::Slash) => {
            let info = parse_slash_tx(tx)?;
            Ok(ParsedTx::Slash(info))
        }
        Ok(BridgeTxType::Unstake) => {
            let info = parse_unstake_tx(tx)?;
            Ok(ParsedTx::Unstake(info))
        }
        Ok(BridgeTxType::DepositRequest | BridgeTxType::Commit) => {
            Err(BridgeTxParseError::UnsupportedTxType(tx.tag().tx_type()))
        }
        Err(unsupported_type) => Err(BridgeTxParseError::UnsupportedTxType(unsupported_type)),
    }
}
