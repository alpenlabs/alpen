use std::fmt::{Debug, Display};

use strata_codec::CodecError;
use strata_msg_fmt::TypeId;
use thiserror::Error;

/// A generic "expected vs actual" error.
#[derive(Debug, Error)]
#[error("expected {expected}, found {actual}")]
pub struct Mismatched<T>
where
    T: Debug + Display,
{
    /// The value that was expected.
    pub expected: T,
    /// The value that was actually encountered.
    pub actual: T,
}

/// Errors that can occur while working with ASM manifest and log types.
#[derive(Debug, Error)]
pub enum AsmManifestError {
    /// Type ID of a decoded section did not match the expected type ID.
    #[error(transparent)]
    TypeIdMismatch(#[from] Mismatched<TypeId>),

    /// Failed to deserialize data for the given TypeId.
    #[error("failed to deserialize TypeId {0:?} data: {1}")]
    TypeIdDeserialization(TypeId, #[source] borsh::io::Error),

    /// Failed to serialize data for the given TypeId.
    #[error("failed to serialize TypeId {0:?} data: {1}")]
    TypeIdSerialization(TypeId, #[source] borsh::io::Error),

    /// Message format error.
    #[error("msgfmt: {0:?}")]
    MsgFmtError(#[from] strata_msg_fmt::Error),

    /// Codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
}

/// Wrapper result type for ASM operations.
pub type AsmManifestResult<T> = Result<T, AsmManifestError>;
