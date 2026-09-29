//! Error types for snark account types.

use thiserror::Error;

/// Errors that can occur when working with update outputs.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OutputsError {
    /// The transfer count exceeds the destination capacity.
    #[error("update has {actual} transfers, exceeding limit {limit}")]
    TransfersCapacityExceeded { actual: usize, limit: usize },

    /// The message count exceeds the destination capacity.
    #[error("update has {actual} messages, exceeding limit {limit}")]
    MessagesCapacityExceeded { actual: usize, limit: usize },
}
