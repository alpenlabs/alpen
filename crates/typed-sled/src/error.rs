use sled::{Error as SledError, transaction::TransactionError};

use crate::CodecError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Codec error
    #[error("Codec Error: {0}")]
    CodecError(#[from] CodecError),

    /// Sled database error
    #[error("Database error: {0}")]
    SledError(#[from] SledError),

    /// Sled transaction error
    #[error("Db transaction error: {0}")]
    TransactionError(#[from] TransactionError),
}

pub type Result<T> = core::result::Result<T, Error>;
