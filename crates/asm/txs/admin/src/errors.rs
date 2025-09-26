use strata_l1_txfmt::TxType;
use strata_l1tx::envelope::parser::EnvelopeParseError;
use thiserror::Error;

/// Top-level error type for the administration subprotocol, composed of smaller error categories.
#[derive(Debug, Error)]
pub enum AdministrationTxParseError {
    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("failed to deserialize transaction for tx_type = {0}")]
    MalformedTransaction(TxType),

    /// Failed to parse the transaction envelope.
    #[error("failed to parse transaction envelope: {0}")]
    MalformedEnvelope(#[from] EnvelopeParseError),

    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("tx type is not defined")]
    UnknownTxType,
}
