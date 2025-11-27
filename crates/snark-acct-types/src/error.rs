//! Error types for snark account types.

use thiserror::Error;

/// Errors that can occur when working with update outputs.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OutputsError {
    /// Attempted to extend transfers beyond the maximum capacity.
    #[error("transfers capacity would be exceeded")]
    TransfersCapacityExceeded,

    /// Attempted to extend messages beyond the maximum capacity.
    #[error("messages capacity would be exceeded")]
    MessagesCapacityExceeded,
}
