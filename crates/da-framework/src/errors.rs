//! DA framework errors.

use thiserror::Error;

/// Errors involved in applying DA.
#[derive(Debug, Error)]
pub enum DaError {
    #[error("context missing required data")]
    InsufficientContext,

    #[error("invalid state diff: {0}")]
    InvalidStateDiff(&'static str),

    #[error("invalid ledger diff: {0}")]
    InvalidLedgerDiff(&'static str),
}

/// Errors generated in building a diff.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("tried to set a value out of allowed bounds")]
    OutOfBoundsValue,

    #[error("not yet implemented")]
    Unimplemented,
}
