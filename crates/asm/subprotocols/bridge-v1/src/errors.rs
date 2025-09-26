use std::fmt::Debug;

use bitcoin::ScriptBuf;
use strata_asm_txs_bridge_v1::errors::{
    DepositTxParseError, DepositValidationError, Mismatch, WithdrawalParseError,
};
use strata_l1_txfmt::TxType;
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, BitcoinTxid},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeSubprotocolError {
    #[error("failed to parse deposit tx")]
    DepositTxParse(#[from] DepositTxParseError),

    #[error("failed to parse deposit tx")]
    DepositTxProcess(#[from] DepositValidationError),

    #[error("failed to parse withdrawal fulfillment tx")]
    WithdrawalTxParse(#[from] WithdrawalParseError),

    #[error("failed to parse withdrawal fulfillment tx")]
    WithdrawalTxProcess(#[from] WithdrawalValidationError),

    #[error("unsupported tx type {0}")]
    UnsupportedTxType(TxType),
}

/// Errors that can occur when validating withdrawal fulfillment transactions.
///
/// When these validation errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to validate.
#[derive(Debug, Error)]
pub enum WithdrawalValidationError {
    /// No assignment found for the deposit
    #[error("No assignment found for deposit index {deposit_idx}")]
    NoAssignmentFound { deposit_idx: u32 },

    /// Operator performing withdrawal doesn't match assigned operator
    #[error("Operator mismatch {0}")]
    OperatorMismatch(Mismatch<OperatorIdx>),

    /// Deposit txid in withdrawal doesn't match the actual deposit
    #[error("Deposit txid mismatch {0}")]
    DepositTxidMismatch(Mismatch<BitcoinTxid>),

    /// Withdrawal amount doesn't match assignment amount
    #[error("Withdrawal amount mismatch {0}")]
    AmountMismatch(Mismatch<BitcoinAmount>),

    /// Withdrawal destination doesn't match assignment destination
    #[error("Withdrawal destination mismatch {0}")]
    DestinationMismatch(Mismatch<ScriptBuf>),
}

/// Errors that can occur when processing withdrawal commands.
///
/// These errors indicate critical system issues that require investigation.
/// Unlike parsing errors, these failures suggest broken system invariants.
#[derive(Debug, Error)]
pub enum WithdrawalCommandError {
    /// No unassigned deposits are available for processing
    #[error("No unassigned deposits available for withdrawal command processing")]
    NoUnassignedDeposits,

    /// No eligible operators found for the deposit
    #[error(
        "No current multisig operator found in deposit's notary operators for deposit index {deposit_idx}"
    )]
    NoEligibleOperators { deposit_idx: u32 },

    /// Deposit amount doesn't match withdrawal command total value
    #[error("Deposit amount mismatch {0}")]
    DepositWithdrawalAmountMismatch(Mismatch<u64>),
}
