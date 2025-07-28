use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, BitcoinTxid},
};
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum DepositError {
    /// The auxiliary data in the deposit transaction tag is malformed or has insufficient length.
    /// Expected at least 37 bytes (4 bytes deposit index + 32 bytes tapscript root + 1+ bytes
    /// destination address).
    #[error(
        "Invalid deposit auxiliary data: expected at least 37 bytes (deposit index + tapscript root + destination), got {0} bytes"
    )]
    InvalidAuxiliaryData(usize),

    /// The transaction type byte in the tag does not match the expected deposit transaction type.
    #[error("Invalid transaction type: expected deposit transaction type {expected}, got {actual}")]
    InvalidTxType { expected: u8, actual: u8 },

    /// Transaction is missing the required deposit output at the expected index.
    #[error(
        "Missing deposit output at index {0}: deposit transactions must have exactly 2 outputs (OP_RETURN tag + P2TR deposit)"
    )]
    MissingOutput(u32),

    /// Signature validation failed during deposit verification.
    /// This indicates the transaction was not signed by the expected operator set.
    #[error("Deposit signature validation failed: {reason}")]
    InvalidSignature { reason: String },

    /// The deposit amount does not match the expected amount for this bridge configuration.
    #[error("Invalid deposit amount: expected {expected} satoshis, got {actual} satoshis")]
    InvalidDepositAmount { expected: u64, actual: u64 },

    /// A deposit with this index already exists in the deposits table.
    /// This should not occur since deposit indices are guaranteed unique by the N/N multisig.
    #[error("Deposit index {0} already exists in deposits table")]
    DepositIdxAlreadyExists(u32),

    /// Failed to create deposit in the deposits table.
    #[error("Failed to create deposit with index {0}: deposit already exists")]
    DepositCreationFailed(u32),

    /// Cannot create deposit entry with empty operators list.
    /// Each deposit must have at least one notary operator.
    #[error("Cannot create deposit entry with empty operators: each deposit must have at least one notary operator")]
    EmptyOperators,
}

#[derive(Debug, Error)]
pub enum WithdrawalParseError {
    /// Transaction has insufficient outputs
    #[error("Transaction has insufficient outputs: expected at least 2, got {0}")]
    InsufficientOutputs(usize),

    /// Metadata script size mismatch
    #[error("Metadata script size mismatch: expected {expected}, got {actual}")]
    InvalidMetadataSize { expected: usize, actual: usize },

    /// Invalid tag bytes conversion
    #[error("Tag bytes conversion error: expected 4 bytes, got {0}")]
    InvalidTagBytes(usize),

    /// Tag mismatch
    #[error("Tag mismatch: expected {expected}, got {actual}")]
    TagMismatch { expected: String, actual: String },

    /// Invalid operator index bytes
    #[error("Operator index bytes conversion error: expected 4 bytes, got {0}")]
    InvalidOperatorIdxBytes(usize),

    /// Invalid deposit index bytes
    #[error("Deposit index bytes conversion error: expected 4 bytes, got {0}")]
    InvalidDepositIdxBytes(usize),

    /// Invalid deposit txid bytes
    #[error("Deposit txid bytes conversion error: expected 32 bytes, got {0}")]
    InvalidDepositTxidBytes(usize),
}

#[derive(Debug, Error)]
pub enum WithdrawalValidationError {
    /// No assignment found for the deposit
    #[error("No assignment found for deposit index {deposit_idx}")]
    NoAssignmentFound { deposit_idx: u32 },

    /// Deposit not found in deposits table
    #[error("Deposit not found for deposit index {deposit_idx}")]
    DepositNotFound { deposit_idx: u32 },

    /// Operator performing withdrawal doesn't match assigned operator
    #[error("Operator mismatch: expected {expected}, got {actual}")]
    OperatorMismatch {
        expected: OperatorIdx,
        actual: OperatorIdx,
    },

    /// Deposit txid in withdrawal doesn't match the actual deposit
    #[error("Deposit txid mismatch: expected {expected:?}, got {actual:?}")]
    DepositTxidMismatch {
        expected: BitcoinTxid,
        actual: BitcoinTxid,
    },

    /// Withdrawal amount doesn't match assignment amount
    #[error("Withdrawal amount mismatch: expected {expected}, got {actual}")]
    AmountMismatch {
        expected: BitcoinAmount,
        actual: BitcoinAmount,
    },
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

    /// Deposit not found for the given index
    #[error("Deposit not found for index {deposit_idx}")]
    DepositNotFound { deposit_idx: u32 },
}
