use alloy_rpc_types_engine::ForkchoiceState;
use async_trait::async_trait;
use thiserror::Error;

/// Interface for interacting with an execution engine that processes payloads
/// and tracks consensus state. Typically wraps an Engine API-compliant client.
#[async_trait]
pub trait ExecutionEngine<TEnginePayload> {
    /// Submits an execution payload to the engine for processing.
    async fn submit_payload(&self, payload: TEnginePayload) -> Result<(), ExecutionEngineError>;

    /// Updates the engine's fork choice state (head, safe, and finalized blocks).
    async fn update_consensus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError>;
}

/// Errors that can occur when interacting with an execution engine.
#[derive(Debug, Error)]
pub enum ExecutionEngineError {
    /// Failed to submit a payload to the engine via `newPayload`.
    #[error("payload submission failed: {0}")]
    PayloadSubmission(String),

    /// Failed to update fork choice state via `forkchoiceUpdated`.
    #[error("fork choice update failed: {0}")]
    ForkChoiceUpdate(String),

    /// Engine rejected the payload as invalid.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    /// Engine is not synchronized or unavailable.
    #[error("engine syncing or unavailable: {0}")]
    EngineSyncing(String),

    /// Communication error with the engine.
    #[error("engine communication error: {0}")]
    Communication(String),

    /// Other unspecified engine error.
    #[error("engine error: {0}")]
    Other(String),
}

impl ExecutionEngineError {
    /// Creates a payload submission error.
    pub fn payload_submission(msg: impl Into<String>) -> Self {
        Self::PayloadSubmission(msg.into())
    }

    /// Creates a fork choice update error.
    pub fn fork_choice_update(msg: impl Into<String>) -> Self {
        Self::ForkChoiceUpdate(msg.into())
    }

    /// Creates an invalid payload error.
    pub fn invalid_payload(msg: impl Into<String>) -> Self {
        Self::InvalidPayload(msg.into())
    }

    /// Creates an engine syncing error.
    pub fn engine_syncing(msg: impl Into<String>) -> Self {
        Self::EngineSyncing(msg.into())
    }

    /// Creates a communication error.
    pub fn communication(msg: impl Into<String>) -> Self {
        Self::Communication(msg.into())
    }

    /// Creates a generic engine error.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
