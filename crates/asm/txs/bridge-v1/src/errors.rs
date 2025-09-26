use std::fmt::Debug;

use strata_l1_txfmt::TxType;
use thiserror::Error;

use crate::{
    constants::{DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    deposit::MIN_DEPOSIT_TX_AUX_DATA_LEN,
    withdrawal_fulfillment::WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN,
};

/// A generic "expected vs got" error.
#[derive(Debug, Error, Clone)]
#[error("(expected {expected:?}, got {got:?})")]
pub struct Mismatch<T>
where
    T: Debug + Clone,
{
    /// The value that was expected.
    pub expected: T,
    /// The value that was actually encountered.
    pub got: T,
}

/// Errors that can occur when parsing deposit transactions.
///
/// When these parsing errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to parse.
#[derive(Debug, Error, Clone)]
pub enum DepositTxParseError {
    /// The auxiliary data in the deposit transaction tag has insufficient length.
    #[error(
        "Auxiliary data too short: expected at least {MIN_DEPOSIT_TX_AUX_DATA_LEN} bytes, got {0} bytes"
    )]
    InvalidAuxiliaryData(usize),

    /// The transaction type byte in the tag does not match the expected deposit transaction type.
    #[error("Invalid transaction type: expected type to be {DEPOSIT_TX_TYPE}, got {0}")]
    InvalidTxType(TxType),

    /// Transaction is missing the required P2TR deposit output at index 1.
    #[error("Missing P2TR deposit output")]
    MissingDepositOutput,
}

/// Errors that can occur when validating deposit transactions.
///
/// When these validation errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to validate.
#[derive(Debug, Error, Clone)]
pub enum DepositValidationError {
    /// Signature validation failed during deposit verification.
    /// This indicates the transaction was not signed by the expected operator set.
    #[error("Deposit signature validation failed: {reason}")]
    InvalidSignature { reason: String },

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

/// Errors that can occur when parsing withdrawal fulfillment transactions.
///
/// When these parsing errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to parse.
#[derive(Debug, Error)]
pub enum WithdrawalParseError {
    /// The auxiliary data in the withdrawal fulfillment transaction doesn't have correct length.
    #[error(
        "Invalid auxiliary data: expected {WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN} bytes, got {0} bytes"
    )]
    InvalidAuxiliaryData(usize),

    /// The transaction type byte in the tag does not match the expected withdrawal fulfillment
    /// transaction type.
    #[error("Invalid transaction type: expected type to be {WITHDRAWAL_TX_TYPE}, got {0}")]
    InvalidTxType(TxType),

    #[error("Transaction is missing output that fulfilled user withdrawal request")]
    MissingUserFulfillmentOutput,
}
