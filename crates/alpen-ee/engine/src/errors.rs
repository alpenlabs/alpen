//! Error module for the engine crate.

use alpen_ee_common::{ExecutionEngineError, StorageError};
use reth_provider::ProviderError;
use strata_acct_types::Hash;
use thiserror::Error;

/// Errors that can occur during chainstate sync.
#[derive(Debug, Error)]
pub enum SyncError {
    /// Missing exec block at height.
    #[error("missing exec block at height {0}")]
    MissingExecBlock(u64),

    /// Missing block payload for specified block hash.
    #[error("missing block payload for hash {0:?}")]
    MissingBlockPayload(Hash),

    /// Block was reported as unfinalized but not found in storage.
    #[error("unfinalized block {0:?} not found in storage")]
    UnfinalizedBlockNotFound(Hash),

    /// Finalized chain is empty.
    #[error("finalized chain is empty")]
    EmptyFinalizedChain,

    /// Storage error.
    #[error("failure in storage: {0}")]
    Storage(#[from] StorageError),

    /// Alpen's execution engine error.
    #[error("failure in execution engine: {0}")]
    Engine(#[from] ExecutionEngineError),

    /// Reth `Provider` error.
    #[error("failure in Reth provider: {0}")]
    Provider(#[from] ProviderError),

    /// Payload deserialization error.
    #[error("failure in payload deserialization: {0}")]
    PayloadDeserialization(String),
}
