use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// No state found for the requested slot.
    #[error("state not found for slot {0}")]
    StateNotFound(u64),

    /// Attempted to store a slot that would create a gap in the stored sequence.
    #[error("missing slot: attempted to store slot {attempted_slot} but last stored slot is {last_slot}")]
    MissingSlot { attempted_slot: u64, last_slot: u64 },

    /// Database operation failed.
    #[error("database error: {0}")]
    Database(String),

    /// Failed to serialize data.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Failed to deserialize data.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Other unspecified error.
    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl StorageError {
    /// Creates a database error.
    pub fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    /// Creates a serialization error.
    pub fn serialization(msg: impl Into<String>) -> Self {
        Self::Serialization(msg.into())
    }

    /// Creates a deserialization error.
    pub fn deserialization(msg: impl Into<String>) -> Self {
        Self::Deserialization(msg.into())
    }
}
