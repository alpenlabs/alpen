use thiserror::Error;

/// Newtype for exec context results, for brevity.
pub type ExecResult<T> = Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not yet implemented")]
    Unimplemented,
}
