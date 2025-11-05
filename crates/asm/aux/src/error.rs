//! Error types for the auxiliary framework.

use strata_asm_common::AsmManifestHash;
use thiserror::Error;

use crate::types::L1TxIndex;

/// Errors that can occur when resolving auxiliary data.
#[derive(Debug, Error)]
pub enum AuxError {
    /// Invalid MMR proof for a manifest leaf.
    ///
    /// This occurs when the provided MMR proof doesn't verify against
    /// the manifest hash, indicating either corrupted data or an invalid
    /// proof from the auxiliary data provider.
    #[error("invalid MMR proof for block height {height}, hash {hash:?}")]
    InvalidMmrProof {
        /// The L1 block height where verification failed
        height: u64,
        /// The manifest hash that failed verification
        hash: AsmManifestHash,
    },

    /// Response content does not match the provided request spec.
    ///
    /// For example, the response range or txid differs from what was
    /// requested via `AuxRequestSpec`.
    #[error("response mismatch for tx {tx_index}: {details}")]
    SpecMismatch {
        /// The transaction index with the mismatch
        tx_index: L1TxIndex,
        /// Human-readable description of the mismatch
        details: String,
    },

    /// Type mismatch between requested and provided auxiliary data.
    ///
    /// This occurs when a subprotocol requests one type of auxiliary data
    /// (e.g., `ManifestLeaves`) but receives a different type (e.g., `BitcoinTx`).
    #[error("type mismatch for tx {tx_index}: expected {expected}, found {found}")]
    TypeMismatch {
        /// The transaction index with the mismatch
        tx_index: L1TxIndex,
        /// The expected response type
        expected: &'static str,
        /// The actual response type received
        found: &'static str,
    },

    /// No auxiliary responses found for the requested transaction.
    ///
    /// This is typically not an error condition - it just means no auxiliary
    /// data was requested for this transaction during pre-processing.
    #[error("no auxiliary responses for tx index {tx_index}")]
    MissingResponse {
        /// The transaction index with no responses
        tx_index: L1TxIndex,
    },
}

/// Result type alias for auxiliary operations.
pub type AuxResult<T> = Result<T, AuxError>;
