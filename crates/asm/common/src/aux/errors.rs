//! Error types for the auxiliary framework.

use bitcoin::Txid;
use thiserror::Error;

use crate::Hash32;

/// Result type alias for auxiliary operations.
pub type AuxResult<T> = Result<T, AuxError>;

/// Errors that can occur during auxiliary data operations.
#[derive(Debug, Error)]
pub enum AuxError {
    /// Invalid MMR proof during initialization.
    ///
    /// Occurs during provider initialization when a provided MMR proof
    /// doesn't verify against the manifest hash.
    #[error("invalid MMR proof at index {index}, hash {hash:?}")]
    InvalidMmrProof {
        /// The index in the batch where verification failed
        index: u64,
        /// The manifest hash that failed verification
        hash: Hash32,
    },

    /// Failed to decode raw Bitcoin transaction during initialization.
    ///
    /// Occurs during provider initialization when a raw transaction
    /// cannot be deserialized.
    #[error("invalid Bitcoin transaction at index {index}: {source}")]
    InvalidBitcoinTx {
        /// The index in the batch where decoding failed
        index: usize,
        /// Underlying decode error
        #[source]
        source: bitcoin::consensus::encode::Error,
    },

    /// Bitcoin transaction not found by txid.
    #[error("Bitcoin transaction not found: {txid:?}")]
    BitcoinTxNotFound {
        /// The requested txid
        txid: Txid,
    },

    /// Manifest leaf not found at the given MMR index.
    #[error("manifest leaf not found at index {index}")]
    ManifestLeafNotFound {
        /// The requested MMR index
        index: u64,
    },
}
