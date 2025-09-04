use thiserror::Error;

/// Top-level error type for the administration subprotocol, composed of smaller error categories.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum AdministrationTxParseError {
    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("failed to deserialize transaction for tx_type = {0}")]
    MalformedTransaction(u8),

    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("tx type is not defined")]
    UnknownTxType,
}
