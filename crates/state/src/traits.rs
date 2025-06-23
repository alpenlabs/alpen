use thiserror::Error;

use crate::chain_state::Chainstate;

#[derive(Debug, Error)]
pub enum StateUpdateError {
    #[error("failed to extract state update from buffer: {0}")]
    FailedExtraction(String),

    #[error("failed to apply update to chainstate: {0}")]
    FailedApplication(String),
}

/// This is any representation of data extracted from raw bytes (usually checkpoint data) that
/// implements a method to update previous chainstate data.
pub trait ChainstateUpdate {
    /// Apply state update to chainstate to get new chainstate.
    fn apply_to_chainstate(&self, chainstate: &mut Chainstate) -> Result<(), StateUpdateError>;

    /// Extract update structure from buffer.
    fn from_buf(buf: &[u8]) -> Result<Self, StateUpdateError>
    where
        Self: Sized;
}
