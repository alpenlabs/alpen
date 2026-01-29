//! Error types for chunked envelope operations.

use bitcoin::secp256k1::Error as Secp256k1Error;
use strata_db_types::DbError;
use thiserror::Error;

/// Errors that can occur during chunked envelope operations.
#[derive(Debug, Error)]
pub enum ChunkedEnvelopeError {
    /// Payload exceeds maximum allowed size.
    #[error("payload too large: {size} bytes exceeds maximum of {max} bytes")]
    PayloadTooLarge {
        /// Actual payload size.
        size: usize,
        /// Maximum allowed size.
        max: usize,
    },

    /// Not enough UTXOs available to fund the transactions.
    #[error("insufficient UTXOs: need {required} sats, have {available} sats")]
    InsufficientUtxos {
        /// Required amount in satoshis.
        required: u64,
        /// Available amount in satoshis.
        available: u64,
    },

    /// Database operation failed.
    #[error("database error: {0}")]
    Database(#[from] DbError),

    /// Transaction building failed.
    #[error("transaction building error: {0}")]
    TxBuild(String),

    /// Signing failed.
    #[error("signing error: {0}")]
    Signing(String),

    /// Payload not found in database.
    #[error("payload not found: {0}")]
    NotFound(String),

    /// Invalid chunk header.
    #[error("invalid chunk header: {0}")]
    InvalidHeader(&'static str),

    /// Max retries exhausted for chunk publication.
    #[error("max retries exhausted for chunk {chunk_index}: {reason}")]
    MaxRetriesExhausted {
        /// Index of the chunk that failed.
        chunk_index: u16,
        /// Reason for the failure.
        reason: String,
    },

    /// Other error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl From<Secp256k1Error> for ChunkedEnvelopeError {
    fn from(e: Secp256k1Error) -> Self {
        Self::Signing(e.to_string())
    }
}
