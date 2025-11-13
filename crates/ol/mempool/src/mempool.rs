//! Mempool trait and core operations.

use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{error::MempoolResult, types::MempoolStats};

/// Core trait for mempool operations.
///
/// The mempool accepts opaque transaction blobs, parses and validates them,
/// and provides validated transactions for block assembly.
///
/// # Transaction Ingestion
///
/// Transactions can be ingested in two ways:
/// 1. **Stream-based** (primary): Via an [`OLTxProvider`](crate::OLTxProvider) stream in the
///    mempool's main loop. This is RPC-agnostic and works with any source (RPC, P2P, ZMQ, etc.).
/// 2. **Direct submission**: Via the `submit_transaction()` method for synchronous submission
///    (useful for RPC handlers, testing, or internal use).
pub trait Mempool: Send + Sync {
    /// Submits a raw transaction to the mempool.
    ///
    /// Accepts a raw transaction blob (opaque bytes) which is parsed into an `OLTransaction`,
    /// validated, and stored. Returns the transaction ID if successful.
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::ParseError) - if the transaction blob cannot
    ///   be parsed
    /// - [`MempoolError`](crate::error::MempoolError::DuplicateTransaction) - if the transaction
    ///   already exists
    /// - [`MempoolError`](crate::error::MempoolError::InvalidTransaction) - if validation fails
    /// - [`MempoolError`](crate::error::MempoolError::TransactionTooLarge) - if the transaction
    ///   exceeds size limits
    /// - [`MempoolError`](crate::error::MempoolError::MempoolCountLimitExceeded) - if mempool is
    ///   full
    /// - [`MempoolError`](crate::error::MempoolError::MempoolSizeLimitExceeded) - if mempool size
    ///   limit exceeded
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if persistence fails
    fn submit_transaction(&self, blob: Vec<u8>) -> MempoolResult<OLTxId>;

    /// Retrieves transactions from the mempool for block assembly.
    ///
    /// Returns up to `limit` transactions, ordered by the mempool's ordering policy
    /// (initially FIFO by entry_slot, then by OLTxId as tie-breaker).
    ///
    /// Returns an empty vector if no transactions are available (not an error).
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if database error occurs
    /// - [`MempoolError`](crate::error::MempoolError::Internal) - if internal error occurs
    fn get_transactions(&self, limit: u64) -> MempoolResult<Vec<(OLTxId, OLTransaction)>>;

    /// Removes transactions from the mempool.
    ///
    /// Typically called after transactions have been included in a block.
    /// Returns the list of transaction IDs that were successfully removed.
    /// Already-removed transactions are silently ignored (idempotent operation).
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if database error occurs
    /// - [`MempoolError`](crate::error::MempoolError::Internal) - if internal error occurs
    fn remove_transactions(&self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>>;

    /// Gets statistics about the current mempool state.
    ///
    /// Returns statistics including transaction count, total size, and rejection counts.
    fn stats(&self) -> MempoolResult<MempoolStats>;
}
