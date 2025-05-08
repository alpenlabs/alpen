use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, Error>;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("not yet implemented")]
    Unimplemented,
}
