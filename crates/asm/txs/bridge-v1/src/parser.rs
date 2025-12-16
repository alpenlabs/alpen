use std::convert::TryInto;

use strata_asm_common::TxInputRef;

use crate::{
    constants::BridgeTxType,
    deposit::{DepositInfo, parse_deposit_tx},
    errors::BridgeTxParseError,
    slash::{SlashInfo, parse_slash_tx},
    unstake::{UnstakeInfo, parse_unstake_tx},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, parse_withdrawal_fulfillment_tx},
};

/// Represents a parsed transaction that can be either a deposit or withdrawal fulfillment.
#[derive(Debug)]
pub enum ParsedTx {
    /// A deposit transaction that locks Bitcoin funds in the bridge
    Deposit(DepositInfo),
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
/// information for bridge transactions that are directly processed by the subprotocol.
///
/// # Arguments
///
/// * `tx` - The transaction input reference to parse
///
/// # Returns
///
/// Returns a [`ParsedTx`] variant containing the extracted transaction information:
/// * `Ok(ParsedTx::Deposit)` - For deposit transactions
/// * `Ok(ParsedTx::WithdrawalFulfillment)` - For withdrawal fulfillment transactions
/// * `Ok(ParsedTx::Slash)` - For slash transactions
/// * `Ok(ParsedTx::Unstake)` - For unstake transactions
///
/// # Errors
///
/// Returns [`BridgeTxParseError`] if:
/// - The transaction type is not directly processed (e.g., `DepositRequest` - fetched as auxiliary
///   data)
/// - The transaction type is not supported by the bridge subprotocol (e.g., `Commit`)
/// - The transaction data extraction fails (malformed transaction structure)
pub fn parse_tx<'t>(tx: &'t TxInputRef<'t>) -> Result<ParsedTx, BridgeTxParseError> {
    match tx.tag().tx_type().try_into() {
        Ok(BridgeTxType::Deposit) => {
            let info = parse_deposit_tx(tx)?;
            Ok(ParsedTx::Deposit(info))
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
        Ok(BridgeTxType::DepositRequest) => {
            // DepositRequest transactions are not parsed at this stage.
            // They are requested as auxiliary input during preprocessing when we encounter
            // a BridgeTxType::Deposit transaction, then parsed on-demand using parse_drt().
            Err(BridgeTxParseError::NotDirectlyProcessed(tx.tag().tx_type()))
        }
        Ok(BridgeTxType::Commit) => {
            // Commit transactions are not currently supported by the bridge subprotocol.
            Err(BridgeTxParseError::UnsupportedTxType(tx.tag().tx_type()))
        }
        Err(unsupported_type) => Err(BridgeTxParseError::UnsupportedTxType(unsupported_type)),
    }
}
