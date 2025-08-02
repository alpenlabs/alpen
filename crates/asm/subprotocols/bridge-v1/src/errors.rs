use std::fmt::Debug;

use bitcoin::ScriptBuf;
use strata_l1_txfmt::TxType;
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, BitcoinTxid},
};
use thiserror::Error;

use crate::{
    constants::{DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    txs::{
        deposit::MIN_DEPOSIT_TX_AUX_DATA_LEN,
        withdrawal_fulfillment::WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN,
    },
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
