//! Errors used in sequencer related logic.

use strata_db_types::errors;
use strata_primitives::{epoch::EpochCommitment, prelude::*};
use thiserror::Error;

/// Errors that may occur in sequencer specific workers.
#[derive(Debug, Error)]
pub enum Error {
    /// Chain is not active.
    #[error("chain is not active yet")]
    ChainInactive,

    /// Missing expected chainstate for block index.
    #[error("missing expected chainstate for blockidx {0}")]
    MissingIdxChainstate(u64),

    /// Missing epoch summary.
    #[error("missing summary for epoch {0:?}")]
    MissingEpochSummary(EpochCommitment),

    /// Missing epoch checkpoint.
    #[error("missing checkpoint for epoch {0}")]
    MissingCheckpoint(u64),

    /// Missing L1 block from the database.
    #[error("missing L1 block from database {0}")]
    MissingL1Block(L1BlockId),

    /// Missing L2 block from the database.
    #[error("missing L2 block from database {0}")]
    MissingL2Block(L2BlockId),

    /// L1 block scanned with the wrong epoch.
    #[error("stored L1 block {0:?} scanned using wrong epoch (got {1}, exp {2})")]
    L1ManifestEpochMismatch(L1BlockId, u64, u64),

    /// Malformed epoch.
    #[error("malformed epoch {0:?}")]
    MalformedEpoch(EpochCommitment),

    /// Database error.
    #[error("db: {0}")]
    Db(#[from] errors::DbError),
}
