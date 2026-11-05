//! DA framework errors.

use thiserror::Error;

/// Errors involved in applying DA.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DaError {
    #[error("context missing required data")]
    InsufficientContext,

    #[error("queue front {front} is past its next index {next}")]
    QueueFrontPastNext { front: u64, next: u64 },

    #[error("queue front increment {incr} exceeds the {len} entries the queue holds")]
    QueueFrontOverrun { incr: u64, len: u64 },

    #[error("appending {added} entries at index {next} overflows the queue index type")]
    QueueIndexOverflow { next: u64, added: u64 },
}

/// Errors generated in building a diff.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("tried to set a value out of allowed bounds")]
    OutOfBoundsValue,

    #[error("not yet implemented")]
    Unimplemented,
}
