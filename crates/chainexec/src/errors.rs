use std::error;

use strata_chaintsn::errors::TsnError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error<C = Box<dyn error::Error + Send + Sync>> {
    /// Context-related errors (IO, missing data, etc.)
    /// These indicate system/process issues, not block validation failures
    #[error("context: {0}")]
    Context(C),

    /// Block validation errors - indicate the block itself is invalid
    #[error("computed state root mismatch with block state root")]
    StateRootMismatch,

    #[error("transition: {0}")]
    Transition(#[from] TsnError),

    #[error("not yet implemented")]
    Unimplemented,

    /// Some unexpected error condition happened during block validation
    #[error("unexpected validation failure: {0}")]
    Unexpected(String),
}

impl<C> Error<C> {
    /// Maps the context error type to a different type
    pub fn map_context<D>(self, f: impl FnOnce(C) -> D) -> Error<D> {
        match self {
            Error::Context(c) => Error::Context(f(c)),
            Error::StateRootMismatch => Error::StateRootMismatch,
            Error::Transition(e) => Error::Transition(e),
            Error::Unimplemented => Error::Unimplemented,
            Error::Unexpected(msg) => Error::Unexpected(msg),
        }
    }
}
