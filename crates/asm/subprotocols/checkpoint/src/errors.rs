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
    #[error("invalid auxiliary data: {0}")]
    InvalidAux(#[from] AuxError),
}

/// CheckpointPayload is invalid
#[derive(Debug, Error)]
pub enum InvalidCheckpointPayload {
    /// Predicate verification failed.
    #[error("sequencer predicate verification failed: {0}")]
    SequencerPredicateVerification(PredicateError),

    /// Predicate verification failed.
    #[error("checkpoint predicate verification failed: {0}")]
    CheckpointPredicateVerification(PredicateError),

    /// Checkpoint epoch does not match expected progression.
    ///
    /// Each checkpoint must advance the epoch by exactly 1.
    #[error("invalid epoch: (expected {expected}, got {actual})")]
    InvalidEpoch { expected: Epoch, actual: Epoch },

    /// Checkpoint L1 height regresses below the last verified height.
    ///
    /// A checkpoint may cover the same L1 height as its predecessor (zero L1
    /// progress), but it must never claim a lower height.
    #[error(
        "checkpoint L1 height regresses: new checkpoint covers up to L1 height {new_height}, but previous checkpoint already covered up to L1 height {prev_height}"
    )]
    L1HeightRegresses { prev_height: u32, new_height: u32 },

    /// Checkpoint L1 height exceeds current block.
    ///
    /// This error occurs when a checkpoint claims to have processed L1 blocks
    /// up to a height that is greater than or equal to the L1 block height
    /// currently being applied in the ASM STF. Since the checkpoint transaction
    /// itself is contained in the L1 block at `current_height`, it can only
    /// reference L1 blocks that were processed **before** this block (i.e., up
    /// to `current_height - 1`).
    #[error("checkpoint L1 height {checkpoint_height} exceeds current block {current_height}")]
    CheckpointBeyondL1Tip {
        checkpoint_height: u32,
        current_height: u32,
    },

    /// L2 slot does not advance.
    #[error(
        "L2 slot must advance: new slot {new_slot} is not greater than previous slot {prev_slot}"
    )]
    L2SlotDoesNotAdvance { prev_slot: u64, new_slot: u64 },

    /// Malformed withdrawal destination descriptor
    ///
    /// This error occurs when a withdrawal intent log contains a malformed
    /// destination descriptor. Since user funds have been destroyed on L2,
    /// this prevents the funds from being withdrawn on L1.
    #[error("malformed withdrawal destination descriptor")]
    MalformedWithdrawalDestDesc,

    /// Epoch counter overflow.
    #[error("epoch overflow: verified tip epoch is at maximum value")]
    EpochOverflow,

    /// L1 height counter overflow.
    #[error("L1 height overflow: verified tip L1 height is at maximum value")]
    L1HeightOverflow,
}
