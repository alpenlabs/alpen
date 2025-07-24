use thiserror::Error;

#[derive(Debug, Error)]
pub enum DepositParseError {
    /// The tag data does not match the expected format.
    #[error("Invalid deposit tag data format")]
    InvalidData,

    /// The magic bytes in the tag do not match the expected value.
    #[error("Invalid magic bytes in deposit tag")]
    InvalidMagic,

    /// The deposit index is out of bounds.
    #[error("Deposit index is out of bounds")]
    OutOfBoundsDepositIndex,

    /// The amount of satoshis is invalid (e.g., negative or zero).
    #[error("Invalid amount of satoshis in deposit tag")]
    InvalidSatsAmount,

    /// Invalid destination address length
    #[error("Invalid destination length {0}")]
    InvalidDestLen(u8),

    /// Transaction missing required output at expected index
    #[error("Missing output at index {0}")]
    MissingOutput(u32),

    /// Deposit amount doesn't match expected amount
    #[error("Invalid deposit amount: expected {expected}, got {actual}")]
    InvalidDepositAmount { expected: u64, actual: u64 },

    /// Deposit address doesn't match expected address
    #[error("Invalid deposit address")]
    InvalidDepositAddress,

    /// Signature validation failed
    #[error("Invalid deposit signature")]
    InvalidSignature,
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
