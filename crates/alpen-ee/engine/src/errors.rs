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

    /// The finalized chain has a best block but no readable local anchor.
    #[error("finalized chain has no readable local anchor")]
    MissingFinalizedChainAnchor,

    /// The local finalized-chain bounds are inconsistent.
    #[error(
        "invalid finalized chain bounds: first retained height {first_height} exceeds best height \
         {best_height}"
    )]
    InvalidFinalizedChainBounds { first_height: u64, best_height: u64 },

    /// A trusted non-genesis finalized anchor is absent from the paired Reth database.
    #[error("trusted finalized anchor at height {height} with hash {hash:?} is missing from Reth")]
    FinalizedAnchorMissingInEngine { height: u64, hash: Hash },

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
