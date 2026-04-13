//! Error types.

pub type ProverResult<T> = Result<T, ProverError>;

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task already exists: {0}")]
    TaskAlreadyExists(String),

    #[error("transient: {0}")]
    TransientFailure(String),

    #[error("permanent: {0}")]
    PermanentFailure(String),

    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl ProverError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientFailure(_))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::PermanentFailure(_))
    }
}
