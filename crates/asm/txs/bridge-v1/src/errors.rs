use std::fmt::Debug;

use strata_codec::CodecError;
use strata_l1_txfmt::TxType;
use thiserror::Error;

use crate::deposit_request::MIN_DRT_AUX_DATA_LEN;

/// Errors that can occur when parsing bridge transaction
#[derive(Debug, Error)]
pub enum BridgeTxParseError {
    #[error("failed to parse deposit tx")]
    DepositTxParse(#[from] DepositTxParseError),

    #[error("failed to parse withdrawal fulfillment tx")]
    WithdrawalTxParse(#[from] WithdrawalParseError),

    #[error("failed to parse slash tx")]
    SlashTxParse(#[from] SlashTxParseError),

    #[error("failed to parse unstake tx")]
    UnstakeTxParse(#[from] UnstakeTxParseError),

    #[error("unsupported tx type {0}")]
    UnsupportedTxType(TxType),
}

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

/// Errors that can occur when building deposit request transactions (DRT).
#[derive(Debug, Error, Clone)]
pub enum DepositRequestBuildError {
    #[error("SPS-50 format error: {0}")]
    TxFmt(String),
}

/// Errors that can occur when parsing deposit request transactions (DRT).
#[derive(Debug, Error, Clone)]
pub enum DepositRequestParseError {
    /// The transaction type byte in the tag does not match the expected deposit request type.
    #[error("Invalid transaction type: expected {expected}, got {actual}")]
    InvalidTxType { actual: u8, expected: u8 },

    /// The auxiliary data in the deposit request transaction tag has insufficient length.
    #[error(
        "Invalid auxiliary data length: expected at least {MIN_DRT_AUX_DATA_LEN} bytes, got {0} bytes"
    )]
    InvalidAuxiliaryData(usize),

    /// Transaction is missing the required P2TR deposit request output at index 1.
    #[error("Missing P2TR deposit request output at index 1")]
    MissingDRTOutput,

    /// OP_RETURN output missing or not at index 0 as required by spec.
    #[error("OP_RETURN output must be at index 0")]
    NoOpReturnOutput,

    /// Failed to parse the SPS-50 transaction format.
    #[error("Failed to parse SPS-50 transaction: {0}")]
    Sps50ParseError(String),
}

/// Errors that can occur when parsing deposit transactions.
///
/// When these parsing errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to parse.
#[derive(Debug, Error)]
pub enum DepositTxParseError {
    /// The auxiliary data in the deposit transaction tag is invalid.
    #[error("Invalid auxiliary data: {0}")]
    InvalidAuxiliaryData(#[from] CodecError),

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

    /// Missing deposit output at the expected index.
    #[error("Missing deposit output at index {0}")]
    MissingDepositOutput(usize),
}

/// Errors that can occur when parsing withdrawal fulfillment transactions.
///
/// When these parsing errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to parse.
#[derive(Debug, Error)]
pub enum WithdrawalParseError {
    /// The auxiliary data in the withdrawal fulfillment transaction is invalid.
    #[error("Invalid auxiliary data: {0}")]
    InvalidAuxiliaryData(#[from] CodecError),

    /// Transaction is missing output that fulfilled user withdrawal request.
    #[error("Transaction is missing output that fulfilled user withdrawal request")]
    MissingUserFulfillmentOutput,
}

/// Errors that can occur when parsing commit transactions.
///
/// When these parsing errors occur, they are logged and the transaction is skipped.
/// No further processing is performed on transactions that fail to parse.
#[derive(Debug, Error)]
pub enum CommitParseError {
    /// The auxiliary data in the commit transaction is invalid.
    #[error("Invalid auxiliary data: {0}")]
    InvalidAuxiliaryData(#[from] CodecError),

    /// The commit transaction does not have the expected number of inputs.
    #[error("Invalid input count: {0}")]
    InvalidInputCount(Mismatch<usize>),

    /// Missing N/N output at index 1.
    #[error("Missing N/N output at index 1")]
    MissingNnOutput,
}

/// Errors that can occur when parsing slash transaction.
#[derive(Debug, Error)]
pub enum SlashTxParseError {
    /// The auxiliary data in the slash transaction is invalid
    #[error("Invalid auxiliary data")]
    InvalidAuxiliaryData(#[from] CodecError),

    #[error("Missing input at index {0}")]
    MissingInput(usize),
}

/// Errors that can occur when parsing unstake transaction.
#[derive(Debug, Error)]
pub enum UnstakeTxParseError {
    /// The auxiliary data in the unstake transaction is invalid
    #[error("Invalid auxiliary data")]
    InvalidAuxiliaryData(#[from] CodecError),

    #[error("Missing input at index {0}")]
    MissingInput(usize),

    /// Stake connector witness is missing the script leaf.
    #[error("Missing stake connector script in witness")]
    MissingStakeScript,

    /// Could not parse the N/N pubkey from the stake connector script.
    #[error("Invalid N/N pubkey in stake connector script")]
    InvalidNnPubkey,

    /// Witness length did not match the expected layout for a stake-connector spend.
    #[error("Invalid stake connector witness length: expected {expected}, got {actual}")]
    InvalidStakeWitnessLen { expected: usize, actual: usize },

    /// Stake connector script does not match expected pattern.
    #[error("Invalid stake connector script structure")]
    InvalidStakeScript,
}
