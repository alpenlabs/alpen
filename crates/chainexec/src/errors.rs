use thiserror::Error;

/// Newtype for exec context results, for brevity.
pub type ExecResult<T> = Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("transition: {0}")]
    Transition(#[from] strata_chaintsn::errors::TsnError),

    #[error("computed state root mismatch with block state root")]
    StateRootMismatch,

    #[error("not yet implemented")]
    Unimplemented,
}
