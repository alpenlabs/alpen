use std::fmt::Debug;

use bitcoin::ScriptBuf;
use strata_asm_common::AuxError;
use strata_asm_txs_bridge_v1::errors::{
    DepositOutputError, DepositTxParseError, DrtSignatureError, Mismatch, SlashTxParseError,
    WithdrawalParseError,
};
use strata_bridge_types::OperatorIdx;
use strata_l1_txfmt::TxType;
use strata_primitives::l1::BitcoinAmount;
use thiserror::Error;

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

    #[error("failed to parse slash tx")]
    SlashTxParse(#[from] SlashTxParseError),

    #[error("failed to validate slash tx")]
    SlashTxValidation(#[from] SlashValidationError),

    #[error("failed to get proper aux data")]
    Aux(#[from] AuxError),

    #[error("unsupported tx type {0}")]
    UnsupportedTxType(TxType),
}

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

/// Errors that can occur when validating withdrawal fulfillment transactions.
///
/// When these validation errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to validate.
#[derive(Debug, Error)]
pub enum WithdrawalValidationError {
    /// No assignment found for the deposit
    #[error("No assignment found for deposit index {deposit_idx}")]
    NoAssignmentFound { deposit_idx: u32 },

    /// Withdrawal amount doesn't match assignment amount
    #[error("Withdrawal amount mismatch {0}")]
    AmountMismatch(Mismatch<BitcoinAmount>),

    /// Withdrawal destination doesn't match assignment destination
    #[error("Withdrawal destination mismatch {0}")]
    DestinationMismatch(Mismatch<ScriptBuf>),
}

#[derive(Debug, Error)]
pub enum SlashValidationError {
    /// Stake connector input is not locked to the expected N/N multisig script
    #[error("stake connector not locked to N/N multisig script")]
    InvalidStakeConnectorScript,
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

    /// Deposit amount doesn't match withdrawal command total value
    #[error("Deposit amount mismatch {0}")]
    DepositWithdrawalAmountMismatch(Mismatch<u64>),

    /// Withdrawal assignment operation failed
    #[error("Withdrawal assignment failed")]
    AssignmentError(#[from] WithdrawalAssignmentError),
}

/// Errors that can occur when creating or managing withdrawal assignments.
///
/// These errors indicate issues with operator assignment logic, such as
/// bitmap inconsistencies or invalid state.
#[derive(Debug, Error)]
pub enum WithdrawalAssignmentError {
    /// No eligible operators found for the deposit
    #[error(
        "No current multisig operator found in deposit's notary operators for deposit index {deposit_idx}"
    )]
    NoEligibleOperators { deposit_idx: u32 },

    /// Notary operators and previous assignees bitmaps have mismatched lengths.
    #[error(
        "Notary operators length ({notary_len}) does not match previous assignees length ({previous_len})"
    )]
    MismatchedBitmapLengths {
        notary_len: usize,
        previous_len: usize,
    },

    /// Current active operators bitmap is shorter than notary operators bitmap.
    /// This indicates a system inconsistency since operator indices are only appended.
    #[error(
        "Current active operators bitmap length ({active_len}) is shorter than notary operators length ({notary_len}). This should never happen as operator bitmaps only grow."
    )]
    InsufficientActiveBitmapLength {
        active_len: usize,
        notary_len: usize,
    },

    /// Bitmap operation failed
    #[error("Bitmap operation failed")]
    BitmapError(#[from] BitmapError),
}

/// Error type for OperatorBitmap operations.
#[derive(Debug, Error, PartialEq)]
pub enum BitmapError {
    /// Attempted to set a bit at an index that would create a gap in the bitmap.
    /// Only sequential indices are allowed.
    #[error(
        "Index {index} is out of bounds for sequential bitmap (valid range: 0..={max_valid_index})"
    )]
    IndexOutOfBounds {
        index: OperatorIdx,
        max_valid_index: OperatorIdx,
    },
}
