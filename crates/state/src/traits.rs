use thiserror::Error;

use crate::chain_state::Chainstate;

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("failed to extract chainstate diff from buffer: {0}")]
    FailedExtraction(String),

    #[error("failed to apply diff to chainstate: {0}")]
    FailedApplication(String),
}

/// This is any representation of data extracted from `SignedCheckpoint` that implements a method to
/// update previous chainstate data. Can be further generalized internally to use in contexts
/// other than checkpoint.
pub trait ChainstateDiff {
    /// Apply state update to chainstate to get new chainstate.
    fn apply_to_chainstate(&self, chainstate: &mut Chainstate) -> Result<(), DiffError>;

    /// Extract diff structure from buffer.
    fn from_buf(buf: &[u8]) -> Result<Self, DiffError>
    where
        Self: Sized;
}
