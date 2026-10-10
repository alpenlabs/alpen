//! Error types for snark account types.

use thiserror::Error;

/// Errors that can occur when working with update outputs.
///
/// `actual` is the count requested by the operation. `limit` is the inclusive list
/// capacity: [`crate::UpdateOutputs`] when extending, or
/// [`strata_acct_types::TxEffects`] when converting.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OutputsError {
    /// The transfer count exceeds the list's inclusive capacity.
    #[error("update has {actual} transfers, exceeding limit {limit}")]
    TransfersCapacityExceeded { actual: usize, limit: usize },

    /// The message count exceeds the list's inclusive capacity.
    #[error("update has {actual} messages, exceeding limit {limit}")]
    MessagesCapacityExceeded { actual: usize, limit: usize },
}
