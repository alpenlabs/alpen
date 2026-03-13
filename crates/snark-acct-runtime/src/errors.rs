//! Error types used by the runtime.

use std::{error::Error, fmt::Debug};

use ssz::DecodeError;
use strata_codec::CodecError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProgramError<I: Error> {
    #[error("mismatched pre-state")]
    MismatchedPreState,

    #[error("mismatched post-state")]
    MismatchedPostState,

    #[error("inconsistent message count")]
    InconsistentMessageCount,

    /// Mismatched coinput count between messages and coinputs.
    #[error("mismatched coinput count (expected {expected}, got {actual})")]
    MismatchedCoinputCount { expected: usize, actual: usize },

    /// When the coinput is malformed and cannot even be checked correctly.
    #[error("malformed coinput")]
    MalformedCoinput,

    /// When a coinput is checked to "match" the message and it doesn't match.
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

    #[error("invalid extradata")]
    InvalidExtraData,

    /// When we reach the end of processing and still have some unsatisfied
    /// obligation to verify.
    #[error("obligations unsatisfied after update finished processing")]
    UnsatisfiedObligations,

    /// Error during message processing at a specific index.
    #[error("failed to process message {idx}: {}", AsRef::as_ref(inner))]
    AtMessage {
        idx: usize,
        inner: Box<ProgramError<I>>,
    },

    /// Some other generic codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),

    #[error("ssz decode: {0}")]
    SszDecode(#[from] DecodeError),

    #[error("internal: {0}")]
    Internal(I),
}

impl<I: Error> ProgramError<I> {
    pub fn at_msg(self, idx: usize) -> Self {
        Self::new_at_msg(idx, self)
    }

    pub fn new_at_msg(idx: usize, inner: Self) -> Self {
        Self::AtMessage {
            idx,
            inner: Box::new(inner),
        }
    }
}

pub type ProgramResult<T, I> = Result<T, ProgramError<I>>;
