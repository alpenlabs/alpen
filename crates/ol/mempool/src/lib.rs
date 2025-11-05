//! OL transaction mempool types.
//!
//! Provides types for managing pending OL transactions before they are included in blocks.

use strata_identifiers::OLTxId;

/// Errors that can occur during mempool operations.
#[derive(Debug, thiserror::Error)]
pub enum MempoolError {
    /// Transaction with the given ID doesn't exist.
    #[error("Transaction {0} not found in mempool")]
    TransactionNotFound(OLTxId),
    // TODO add more errors here
}
