use sled::Error as SledError;

use crate::CodecError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Codec error
    #[error("Codec Error: {0}")]
    CodecError(#[from] CodecError),

    /// Sled database error
    #[error("Database error: {0}")]
    SledError(#[from] SledError),
}

pub type Result<T> = core::result::Result<T, Error>;
