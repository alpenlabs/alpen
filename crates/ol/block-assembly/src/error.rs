//! Error types for block assembly operations.

use strata_db_types::errors::DbError;
use strata_ol_mempool::MempoolError;

/// Errors that can occur during block assembly operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockAssemblyError {
    /// Database operation failed.
    #[error("db: {0}")]
    Database(#[from] DbError),

    /// Mempool operation failed.
    #[error("mempool: {0}")]
    Mempool(#[from] MempoolError),

    /// Invalid L1 block range where `from_block` height > `to_block` height.
    #[error("invalid L1 block height range (from {from_height} to {to_height})")]
    InvalidRange { from_height: u64, to_height: u64 },
}
