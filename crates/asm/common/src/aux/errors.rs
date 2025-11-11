//! Error types for the auxiliary framework.

use thiserror::Error;

use crate::{Hash32, L1TxIndex};

/// Result type alias for auxiliary operations.
pub type AuxResult<T> = Result<T, AuxError>;

/// Errors that can occur when resolving auxiliary data.
#[derive(Debug, Error)]
pub enum AuxError {
    /// No auxiliary responses found for the requested transaction.
    ///
    /// This is typically not an error condition - it just means no auxiliary
    /// data was requested for this transaction during pre-processing.
    #[error("no auxiliary responses for tx index {tx_index}")]
    MissingResponse {
        /// The transaction index with no responses
        tx_index: L1TxIndex,
    },

    /// Error occurred while requesting manifest leaves.
    #[error(transparent)]
    ManifestLeaves(#[from] ManifestLeavesError),

    /// Error occurred while requesting Bitcoin transaction.
    #[error(transparent)]
    BitcoinTx(#[from] BitcoinTxError),
}

/// Errors that can occur when requesting manifest leaves.
#[derive(Debug, Error)]
pub enum ManifestLeavesError {
    /// The response length doesn't match the requested height range.
    ///
    /// Occurs when the number of manifest leaves in the response doesn't match
    /// the expected count based on start_height and end_height.
    #[error(
        "manifest leaves length mismatch for tx index {tx_index}: expected {expected}, found {found}"
    )]
    LengthMismatch {
        /// The transaction index being resolved
        tx_index: L1TxIndex,
        /// Expected number of leaves
        expected: usize,
        /// Actual number of leaves received
        found: usize,
    },

    /// The number of proofs doesn't match the number of leaves.
    ///
    /// Occurs when the proofs vector length doesn't equal the leaves vector length,
    /// indicating malformed or incomplete auxiliary data.
    #[error(
        "manifest proofs count mismatch for tx index {tx_index}: expected {expected}, found {found}"
    )]
    ProofsCountMismatch {
        /// The transaction index being resolved
        tx_index: L1TxIndex,
        /// Expected number of proofs (same as leaves count)
        expected: usize,
        /// Actual number of proofs received
        found: usize,
    },

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
        hash: Hash32,
    },
}

/// Errors that can occur when requesting Bitcoin transactions.
#[derive(Debug, Error)]
pub enum BitcoinTxError {
    /// Failed to decode raw Bitcoin transaction bytes.
    ///
    /// Occurs when the provided raw transaction cannot be deserialized
    /// into a valid `bitcoin::Transaction`.
    #[error("invalid Bitcoin transaction for tx index {tx_index}: {source}")]
    InvalidTx {
        /// The transaction index being resolved
        tx_index: L1TxIndex,
        /// Underlying decode error
        #[source]
        source: bitcoin::consensus::encode::Error,
    },

    /// The resolved Bitcoin transaction ID does not match the requested one.
    #[error("Bitcoin txid mismatch: expected {expected:?}, found {found:?}")]
    TxidMismatch {
        /// The requested txid
        expected: [u8; 32],
        /// The txid computed from provided bytes
        found: [u8; 32],
    },
}
