//! OL mempool database interface and its record types.

use serde::{Deserialize, Serialize};
use strata_identifiers::OLTxId;

use crate::DbResult;

/// Stored mempool transaction with ordering metadata.
///
/// Used by [`MempoolDatabase`] trait for storage and retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolTxData {
    /// Transaction ID.
    pub txid: OLTxId,
    /// Raw transaction bytes.
    pub tx_bytes: Vec<u8>,
    /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
    ///
    /// Persists across restarts.
    pub timestamp_micros: u64,
}

impl MempoolTxData {
    /// Create new mempool transaction data.
    pub fn new(txid: OLTxId, tx_bytes: Vec<u8>, timestamp_micros: u64) -> Self {
        Self {
            txid,
            tx_bytes,
            timestamp_micros,
        }
    }
}

/// Database interface for OL mempool transactions.
///
/// Stores transactions as opaque bytes with ordering metadata.
#[cfg_attr(
    feature = "proxies",
    strata_db_macros::gen_proxy(error = crate::DbError, tracing_component = "storage:mempool")
)]
pub trait MempoolDatabase: Send + Sync + 'static {
    /// Store a transaction in the mempool.
    ///
    /// Does not validate that txid matches the transaction bytes.
    fn put_tx(&self, data: MempoolTxData) -> DbResult<()>;

    /// Get a transaction by its ID.
    ///
    /// Returns transaction data if found.
    fn get_tx(&self, txid: OLTxId) -> DbResult<Option<MempoolTxData>>;

    /// Get all transactions in the mempool
    ///
    /// Does not validate or parse transaction format.
    fn get_all_txs(&self) -> DbResult<Vec<MempoolTxData>>;

    /// Delete a transaction from the mempool.
    ///
    /// Returns true if the transaction existed and was deleted, false otherwise.
    fn del_tx(&self, txid: OLTxId) -> DbResult<bool>;
}
