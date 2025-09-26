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

/// Errors that can occur during DRT (Deposit Request Transaction) spending signature validation.
#[derive(Debug, Error, Clone)]
pub enum DrtSignatureError {
    /// No witness data found in the transaction input.
    #[error("No witness data found in transaction input")]
    MissingWitness,

    /// Failed to parse the taproot signature from witness data.
    #[error("Failed to parse taproot signature: {0}")]
    InvalidSignatureFormat(String),

    /// Schnorr signature verification failed against the expected key.
    #[error("Schnorr signature verification failed: {0}")]
    SchnorrVerificationFailed(String),
}

/// Errors that can occur during deposit output lock validation.
#[derive(Debug, Error, Clone)]
pub enum DepositOutputError {
    /// The operator public key is malformed or invalid.
    #[error("Invalid operator public key")]
    InvalidOperatorKey,

    /// The deposit output is not locked to the expected aggregated operator key.
    #[error("Deposit output is not locked to the aggregated operator key")]
    WrongOutputLock,
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
