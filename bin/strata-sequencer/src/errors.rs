use std::io;

use strata_tasks::TaskError;
use thiserror::Error;

#[expect(
    clippy::absolute_paths,
    reason = "Disambiguate from std::result::Result"
)]
pub(crate) type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub(crate) enum AppError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("args: {0}")]
    InvalidArgs(String),

    #[error("task: {0}")]
    Task(#[from] TaskError),

    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),
}
