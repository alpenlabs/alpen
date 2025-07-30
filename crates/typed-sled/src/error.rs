// re-export `ConflictableTransactionError`
pub use sled::transaction::ConflictableTransactionError;
use sled::{
    Error as SledError,
    transaction::{ConflictableTransactionResult, UnabortableTransactionError},
};

use crate::CodecError;

/// The main error type for typed-sled operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Codec error
    #[error("Codec Error: {0}")]
    CodecError(#[from] CodecError),

    /// Sled database error
    #[error("Database error: {0}")]
    SledError(#[from] SledError),

    /// Sled transaction error
    #[error("Transaction error: {0}")]
    TransactionError(#[from] UnabortableTransactionError),
}

impl From<Error> for ConflictableTransactionError<Error> {
    fn from(value: Error) -> Self {
        ConflictableTransactionError::Abort(value)
    }
}

/// A type alias for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;
