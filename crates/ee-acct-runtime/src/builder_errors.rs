//! Error types for builder utilities.

use strata_acct_types::Hash;
use strata_codec::CodecError;
use strata_ee_acct_types::{EnvError, MessageDecodeError};
use strata_snark_acct_runtime::ProgramError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BuilderError {
    /// Codec error during encoding or decoding.
    #[error("codec error")]
    Codec(#[from] CodecError),

    /// Execution environment error.
    #[error("execution environment error")]
    Env(#[from] EnvError),

    /// Message decode error.
    #[error("message decode error")]
    MessageDecode(#[from] MessageDecodeError),

    /// Program error during message processing.
    #[error("snark program: {0}")]
    Program(ProgramError<EnvError>),

    /// Chain linkage mismatch when accepting a chunk transition.
    #[error("chunk parent {parent} does not match current tip {expected}")]
    ChainLinkage { expected: Hash, parent: Hash },

    /// Pending input mismatch when accepting a chunk transition.
    #[error("chunk input at position {position} does not match pending input")]
    InputMismatch { position: usize },

    /// Accumulated output transfers or messages exceeded protocol capacity.
    #[error("output overflow")]
    OutputOverflow,
}

/// Manual impl for this trait due to macro inflexibility, I guess?
impl From<ProgramError<EnvError>> for BuilderError {
    fn from(value: ProgramError<EnvError>) -> Self {
        Self::Program(value)
    }
}

pub type BuilderResult<T> = Result<T, BuilderError>;
