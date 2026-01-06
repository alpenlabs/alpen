//! Error types for checkpoint types.

use thiserror::Error;

/// Error type for checkpoint payload construction.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckpointPayloadError {
    #[error("state diff too large: {provided} bytes (max {max})")]
    StateDiffTooLarge { provided: u64, max: u64 },

    #[error("OL logs too large: {provided} bytes (max {max})")]
    OlLogsTooLarge { provided: u64, max: u64 },

    #[error("proof too large: {provided} bytes (max {max})")]
    ProofTooLarge { provided: u64, max: u64 },
}
