use strata_asm_common::AuxError;
use strata_asm_proto_checkpoint_txs::CheckpointTxError;
use strata_predicate::PredicateError;
use thiserror::Error;

/// Result type for checkpoint subprotocol operations.
pub(crate) type CheckpointResult<T> = Result<T, CheckpointError>;

/// Errors that can occur during checkpoint processing.
#[derive(Debug, Error)]
pub(crate) enum CheckpointError {
    /// Failed to parse checkpoint transaction.
    #[error("checkpoint parsing error: {0}")]
    Parsing(#[from] CheckpointTxError),

    /// Checkpoint signature verification failed.
    #[error("invalid checkpoint signature")]
    InvalidSignature,

    /// Failed to retrieve manifest hashes from auxiliary data.
    #[error("auxiliary data error: {0}")]
    AuxData(#[from] AuxError),

    /// Checkpoint proof verification failed.
    #[error("proof verification failed: {0}")]
    ProofVerification(#[from] PredicateError),
}
