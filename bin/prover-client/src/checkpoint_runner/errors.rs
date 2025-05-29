use thiserror::Error;

use crate::errors::ProvingTaskError;

/// Represents errors that can occur during checkpoint-related operations.
///
/// This error type encompasses various failure scenarios that may occur when
/// interacting with checkpoints, including RPC communication issues,
/// data validation problems, and serialization errors. Each variant provides
/// detailed information about the specific error condition.
#[derive(Error, Debug)]
pub(crate) enum CheckpointError {
    /// Occurs when the RPC request to fetch checkpoint data fails.
    #[error("Failed to fetch checkpoint data: {0}")]
    FetchError(String),

    /// Occurs when no checkpoint data is returned from the sequencer.
    #[error("No checkpoint data returned from sequencer for index {0}")]
    CheckpointNotFound(u64),

    /// Occurs when failed to submit checkpoint proof to the sequencer.
    #[error("Failed to submit checkpoint proof for index {index}: {error}")]
    SubmitProofError { index: u64, error: String },

    /// Occurs when an error is encountered during proof generation
    /// or retrieval of a checkpoint proof, detailed in [`ProvingTaskError`].
    #[error("Proof error: {0}")]
    ProofErr(#[from] ProvingTaskError),
}

/// A type alias for results involving checkpoint operations.
pub(crate) type CheckpointResult<T> = Result<T, CheckpointError>;
