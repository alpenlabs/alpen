//! Error types used by the runtime.

use strata_codec::CodecError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProgramError {
    /// When the coinput is malformed and cannot even be checked correctly.
    #[error("malformed coinput")]
    MalformedCoinput,

    /// When a coinput is checked to "match" the message and it doens't match.
    ///
    /// An example of this is when the message is just the hash of the coinput
    /// expected to be used with it.
    #[error("mismatched coinput")]
    MismatchedCoinput,

    /// When the coinput is just incorrect with respect to the message.
    #[error("invalid coinput")]
    InvalidCoinput,

    #[error("malformed extradata")]
    MalformedExtraData,

    #[error("malformed extradata")]
    InvalidExtraData,

    /// When we reach the end of processing and still have some unsatisfied
    /// obligation to verify.
    #[error("obligations unsatisfied after update finished processing")]
    UnsatisfiedObligations,

    /// Some other generic codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
}

pub type ProgramResult<T> = Result<T, ProgramError>;
