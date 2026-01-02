use borsh::io;
// Re-export error types from manifest-types crate
pub use strata_asm_manifest_types::{AsmManifestError, AsmManifestResult, Mismatched};
use strata_asm_types::L1VerificationError;
use strata_l1_txfmt::SubprotocolId;
use strata_merkle::error::MerkleError;
use thiserror::Error;

/// Convenience result wrapper.
pub type AsmResult<T> = Result<T, AsmError>;

/// Errors that can occur while working with ASM subprotocols.
#[derive(Debug, Error)]
pub enum AsmError {
    /// Subprotocol ID of a decoded section did not match the expected subprotocol ID.
    #[error(transparent)]
    SubprotoIdMismatch(#[from] Mismatched<SubprotocolId>),

    /// The requested subprotocol ID was not found.
    #[error("subproto {0:?} does not exist")]
    InvalidSubprotocol(SubprotocolId),

    /// The requested subprotocol state ID was not found.
    #[error("subproto {0:?} does not exist")]
    InvalidSubprotocolState(SubprotocolId),

    /// Failed to deserialize the state of the given subprotocol.
    #[error("failed to deserialize subprotocol {0} state: {1}")]
    Deserialization(SubprotocolId, #[source] io::Error),

    /// Failed to serialize the state of the given subprotocol.
    #[error("failed to serialize subprotocol {0} state: {1}")]
    Serialization(SubprotocolId, #[source] io::Error),

    /// L1Header do not follow consensus rules.
    #[error("L1Header do not follow consensus rules")]
    InvalidL1Header(#[source] L1VerificationError),

    /// Missing genesis configuration for subprotocol
    #[error("missing genesis configuration for subprotocol {0}")]
    MissingGenesisConfig(SubprotocolId),

    /// Error related to Merkle tree operations
    #[error("merkle tree error: {0}")]
    MerkleError(#[from] MerkleError),

    /// Wrapped error from manifest-types crate
    #[error(transparent)]
    ManifestError(#[from] AsmManifestError),
}
