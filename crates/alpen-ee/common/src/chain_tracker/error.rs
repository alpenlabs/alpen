//! Error types for chain tracking operations.

use std::fmt::Debug;

use thiserror::Error;

/// Errors that can occur during chain tracking operations.
#[derive(Debug, Error)]
pub enum ChainTrackerError<Id: Debug> {
    /// Attempted to finalize an item that is not tracked
    #[error("unknown item for finalization: {0:?}")]
    UnknownItem(Id),

    /// Internal tracker state is inconsistent
    #[error("invalid tracker state")]
    InvalidState,
}
