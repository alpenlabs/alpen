//! Error types for the auxiliary framework.

use bitcoin::{Txid, consensus};
use strata_asm_manifest_types::Hash32;
use thiserror::Error;

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
        source: consensus::encode::Error,
    },

    /// Bitcoin transaction not found.
    #[error("Bitcoin transaction not found: {txid:?}")]
    BitcoinTxNotFound {
        /// The requested txid
        txid: Txid,
    },

    /// Bitcoin [`TxOut`](bitcoin::TxOut) not found.
    #[error("Bitcoin transaction out not found for {txid:?}: {vout:?}")]
    BitcoinTxOutNotFound {
        /// The requested txid
        txid: Txid,
        /// The requested vout
        vout: u32,
    },

    /// Manifest hash not found at the given L1 Block height.
    #[error("manifest hash not found for height {height}")]
    ManifestHashNotFound {
        /// The requested MMR index
        height: u64,
    },
}
