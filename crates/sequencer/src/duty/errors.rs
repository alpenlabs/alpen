//! Common error types for sequencer duty.

use strata_ol_chain_types::L2BlockId;
use thiserror::Error;

/// Errors used in sequencer duty.
#[derive(Debug, Error)]
pub enum Error {
    /// L2 block not found in db.
    #[error("L2 blkid {0:?} missing from database")]
    MissingL2Block(L2BlockId),

    /// Checkpoint missing.
    #[error("missing expected checkpoint {0} in database")]
    MissingCheckpoint(u64),

    /// Other db error.
    #[error("db: {0}")]
    Db(#[from] strata_db::errors::DbError),
}
