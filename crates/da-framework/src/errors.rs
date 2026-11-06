//! DA framework errors.

use thiserror::Error;

/// Errors involved in applying DA.
#[derive(Debug, Error)]
pub enum DaError {
    #[error("context missing required data")]
    InsufficientContext,
}

/// Errors in applying a queue write.
///
/// Its own type, not variants of [`DaError`]: the checkpoint guest compiles
/// every `Result<_, DaError>` of the register, counter and compound writes,
/// so widening that enum moves the guest's verifying key, while nothing in
/// the guest instantiates the queue.
#[derive(Debug, Error)]
pub enum DaQueueError {
    #[error("queue front {front} is past its next index {next}")]
    FrontPastNext { front: u64, next: u64 },

    #[error(
        "front increment {incr} exceeds the {len_with_tail} entries the queue holds once the tail is appended"
    )]
    FrontOverrun { incr: u64, len_with_tail: u64 },

    #[error("appending {added} entries at index {next} overflows the queue index type")]
    IndexOverflow { next: u64, added: u64 },
}

/// Errors generated in building a diff.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("tried to set a value out of allowed bounds")]
    OutOfBoundsValue,

    #[error("not yet implemented")]
    Unimplemented,
}
