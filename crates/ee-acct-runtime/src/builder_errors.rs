//! Error types for test utilities.

use strata_codec::CodecError;
use strata_ee_acct_types::{EnvError, MessageDecodeError};
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

    /// State root mismatch when building a block.
    #[error("state root mismatch")]
    StateRootMismatch,

    /// Not enough pending inputs available.
    #[error("not enough pending inputs: requested {requested}, available {available}")]
    InsufficientInputs { requested: usize, available: usize },

    /// No blocks in chain segment.
    #[error("chain segment has no blocks")]
    EmptyChainSegment,
}

pub type BuilderResult<T> = Result<T, BuilderError>;
