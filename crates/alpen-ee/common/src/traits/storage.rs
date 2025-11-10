use async_trait::async_trait;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use thiserror::Error;

use crate::EeAccountStateAtBlock;

#[derive(Debug)]
pub enum OLBlockOrSlot<'a> {
    Block(&'a OLBlockId),
    Slot(u64),
}

impl<'a> From<&'a OLBlockId> for OLBlockOrSlot<'a> {
    fn from(value: &'a OLBlockId) -> Self {
        Self::Block(value)
    }
}

impl From<u64> for OLBlockOrSlot<'_> {
    fn from(value: u64) -> Self {
        OLBlockOrSlot::Slot(value)
    }
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
/// Persistence for EE Nodes
pub trait Storage {
    /// Get EE account internal state corresponding to a given OL slot.
    async fn ee_account_state<'a>(
        &self,
        block_or_slot: OLBlockOrSlot<'a>,
    ) -> Result<Option<EeAccountStateAtBlock>, StorageError>;

    /// Get EE account internal state for the highest slot available.
    async fn best_ee_account_state(&self) -> Result<Option<EeAccountStateAtBlock>, StorageError>;

    /// Store EE account internal state for next slot.
    async fn store_ee_account_state(
        &self,
        ol_block: &OLBlockCommitment,
        ee_account_state: &EeAccountState,
    ) -> Result<(), StorageError>;

    /// Remove stored EE internal account state for slots > `to_slot`.
    async fn rollback_ee_account_state(&self, to_slot: u64) -> Result<(), StorageError>;
}

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
