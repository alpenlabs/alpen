//! Error types for checkpoint types.

use thiserror::Error;

/// Error type for checkpoint payload construction.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckpointPayloadError {
    #[error("state diff too large: {provided} bytes (max {max})")]
    StateDiffTooLarge { provided: u64, max: u64 },

    #[error("OL logs count too large: {provided} (max {max})")]
    OlLogsTooLarge { provided: u64, max: u64 },

    #[error("OL log payload too large: {provided} bytes (max {max})")]
    OlLogPayloadTooLarge { provided: u64, max: u64 },

    #[error("OL logs total payload too large: {provided} bytes (max {max})")]
    OlLogsTotalPayloadTooLarge { provided: u64, max: u64 },

    #[error("proof too large: {provided} bytes (max {max})")]
    ProofTooLarge { provided: u64, max: u64 },
}
