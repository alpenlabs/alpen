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
