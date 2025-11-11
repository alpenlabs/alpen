use strata_db_types::errors::DbError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BroadcasterError {
    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("client: {0}")]
    Client(#[from] anyhow::Error),

    #[error("expected tx not found in db. Idx {0}")]
    TxNotFound(u64),

    #[error("{0}")]
    Other(String),
}

pub(crate) type BroadcasterResult<T> = Result<T, BroadcasterError>;
