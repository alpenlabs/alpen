use std::fmt::Debug;

use bitcoin::ScriptBuf;
use strata_asm_txs_bridge_v1::errors::{
    DepositTxParseError, DrtSignatureError, DepositOutputError, Mismatch, WithdrawalParseError,
};
use strata_l1_txfmt::TxType;
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, BitcoinTxid},
};
use thiserror::Error;

/// Errors that can occur when validating deposit transactions at the subprotocol level.
///
/// These errors represent state-level validation failures that occur after successful
/// transaction parsing and cryptographic validation.
#[derive(Debug, Error)]
pub enum DepositValidationError {
    /// DRT spending signature validation failed.
    #[error("DRT spending signature validation failed")]
    DrtSignature(#[from] DrtSignatureError),

    /// Deposit output lock validation failed.
    #[error("Deposit output lock validation failed")]
    DepositOutput(#[from] DepositOutputError),

    /// The deposit amount does not match the expected amount for this bridge configuration.
    #[error("Invalid deposit amount")]
    MismatchDepositAmount(Mismatch<u64>),

    /// A deposit with this index already exists in the deposits table.
    /// This should not occur since deposit indices are guaranteed unique by the N/N multisig.
    #[error("Deposit index {0} already exists in deposits table")]
    DepositIdxAlreadyExists(u32),

    /// Cannot create deposit entry with empty operators list.
    /// Each deposit must have at least one notary operator.
    #[error("Cannot create deposit entry with empty operators.")]
    EmptyOperators,
}

#[derive(Debug, Error)]
pub enum BridgeSubprotocolError {
    #[error("failed to parse deposit tx")]
    DepositTxParse(#[from] DepositTxParseError),

    #[error("failed to process deposit tx")]
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
