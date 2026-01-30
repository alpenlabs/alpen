//! Common error types for sequencer duty.

use strata_db_types::errors::DbError;
use strata_identifiers::OLBlockId;
use thiserror::Error;

/// Errors used in sequencer duty.
#[derive(Debug, Error)]
pub enum Error {
    /// OL block not found in db.
    #[error("OL blkid {0:?} missing from database")]
    MissingOLBlock(OLBlockId),

    /// Checkpoint missing.
    #[error("missing expected checkpoint {0} in database")]
    MissingCheckpoint(u64),

    /// Other db error.
    #[error("db: {0}")]
    Db(#[from] DbError),
}
