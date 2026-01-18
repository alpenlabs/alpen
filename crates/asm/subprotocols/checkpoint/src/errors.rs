use strata_asm_common::AuxError;
use strata_identifiers::Epoch;
use strata_predicate::PredicateError;
use thiserror::Error;

/// Result type for checkpoint subprotocol operations.
pub(crate) type CheckpointValidationResult<T> = Result<T, CheckpointValidationError>;

#[derive(Debug, Error)]
pub enum CheckpointValidationError {
    #[error("invalid checkpoint payload: {0}")]
    InvalidPayload(#[from] InvalidCheckpointPayload),

    /// Failed to retrieve manifest hashes from auxiliary data.
    #[error("auxiliary data error: {0}")]
    InvalidAux(#[from] AuxError),
}

/// CheckpointPayload is invalid
#[derive(Debug, Error)]
pub enum InvalidCheckpointPayload {
    /// Predicate verification failed.
    #[error("predicate verification failed: {0}")]
    PredicateVerification(#[from] PredicateError),

    /// Checkpoint epoch does not match expected progression.
    ///
    /// Each checkpoint must advance the epoch by exactly 1.
    #[error("invalid epoch: expected {expected}, got {actual}")]
    InvalidEpoch { expected: Epoch, actual: Epoch },

    /// Checkpoint goes backwards in L1 height.
    ///
    /// This error occurs when a checkpoint claims an L1 height that is before
    /// the L1 height of the previously verified checkpoint. Checkpoints must
    /// advance forward or stay at the same L1 height, never go backwards.
    #[error(
        "checkpoint goes backwards in L1 height: new checkpoint covers up to L1 height {new_height}, but previous checkpoint covered up to L1 height {prev_height}"
    )]
    L1HeightGoesBackwards { prev_height: u32, new_height: u32 },

    /// Checkpoint claims L1 blocks that don't exist yet.
    ///
    /// This error occurs when a checkpoint claims to have processed L1 blocks
    /// up to a height that is greater than or equal to the current L1 chain tip.
    /// Checkpoints can only reference L1 blocks that have already been observed.
    #[error(
        "checkpoint claims unverified L1 blocks: checkpoint claims L1 height {checkpoint_height}, but current verified L1 tip is only at {current_height}"
    )]
    CheckpointBeyondL1Tip {
        checkpoint_height: u32,
        current_height: u32,
    },
}
