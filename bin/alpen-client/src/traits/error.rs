use thiserror::Error;

/// Errors that can occur when interacting with an execution engine.
#[derive(Debug, Error)]
pub(crate) enum ExecutionEngineError {
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
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn payload_submission(msg: impl Into<String>) -> Self {
        Self::PayloadSubmission(msg.into())
    }

    /// Creates a fork choice update error.
    pub(crate) fn fork_choice_update(msg: impl Into<String>) -> Self {
        Self::ForkChoiceUpdate(msg.into())
    }

    /// Creates an invalid payload error.
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn invalid_payload(msg: impl Into<String>) -> Self {
        Self::InvalidPayload(msg.into())
    }

    /// Creates an engine syncing error.
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn engine_syncing(msg: impl Into<String>) -> Self {
        Self::EngineSyncing(msg.into())
    }

    /// Creates a communication error.
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn communication(msg: impl Into<String>) -> Self {
        Self::Communication(msg.into())
    }

    /// Creates a generic engine error.
    #[expect(dead_code, reason = "wip")]
    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
