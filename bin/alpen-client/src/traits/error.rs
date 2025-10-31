use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum OlClientError {
    #[error(
        "invalid slot range: end_slot ({end_slot}) must be greater than start_slot ({start_slot})"
    )]
    InvalidSlotRange { start_slot: u64, end_slot: u64 },

    #[error("unexpected block count: expected {expected} blocks, got {actual}")]
    UnexpectedBlockCount { expected: usize, actual: usize },

    #[error("unexpected operation count: expected {expected} operation lists, got {actual}")]
    UnexpectedOperationCount { expected: usize, actual: usize },

    #[error("unexpected chain status slot order: {latest} >= {confirmed} >= {finalized}")]
    InvalidChainStatusSlotOrder {
        latest: u64,
        confirmed: u64,
        finalized: u64,
    },

    #[error("network error: {0}")]
    Network(String),

    #[error("rpc error: {0}")]
    Rpc(String),

    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl OlClientError {
    pub(crate) fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    pub(crate) fn rpc(msg: impl Into<String>) -> Self {
        Self::Rpc(msg.into())
    }
}

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("state not found for slot {0}")]
    StateNotFound(u64),

    #[error("missing slot: attempted to store slot {attempted_slot} but last stored slot is {last_slot}")]
    MissingSlot { attempted_slot: u64, last_slot: u64 },

    #[error("database error: {0}")]
    Database(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl StorageError {
    pub(crate) fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    pub(crate) fn serialization(msg: impl Into<String>) -> Self {
        Self::Serialization(msg.into())
    }

    pub(crate) fn deserialization(msg: impl Into<String>) -> Self {
        Self::Deserialization(msg.into())
    }
}

/// Errors that can occur when interacting with an execution engine.
#[derive(Debug, Error)]
pub(crate) enum ExecutionEngineError {
    /// Failed to submit a payload to the engine (newPayload).
    #[error("payload submission failed: {0}")]
    PayloadSubmission(String),

    /// Failed to update fork choice state (forkchoiceUpdated).
    #[error("fork choice update failed: {0}")]
    ForkChoiceUpdate(String),

    /// The engine rejected the payload as invalid.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    /// The engine is not synchronized or in a bad state.
    #[error("engine syncing or unavailable: {0}")]
    EngineSyncing(String),

    /// Communication error with the engine (e.g., channel closed, timeout).
    #[error("engine communication error: {0}")]
    Communication(String),

    /// Unknown or unspecified error from the engine.
    #[error("engine error: {0}")]
    Other(String),
}

impl ExecutionEngineError {
    pub(crate) fn payload_submission(msg: impl Into<String>) -> Self {
        Self::PayloadSubmission(msg.into())
    }

    pub(crate) fn fork_choice_update(msg: impl Into<String>) -> Self {
        Self::ForkChoiceUpdate(msg.into())
    }

    pub(crate) fn invalid_payload(msg: impl Into<String>) -> Self {
        Self::InvalidPayload(msg.into())
    }

    pub(crate) fn engine_syncing(msg: impl Into<String>) -> Self {
        Self::EngineSyncing(msg.into())
    }

    pub(crate) fn communication(msg: impl Into<String>) -> Self {
        Self::Communication(msg.into())
    }

    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
