use std::{io, result};

use strata_tasks::TaskError;
use thiserror::Error;

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

pub(crate) type Result<T> = result::Result<T, AppError>;
