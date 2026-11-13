//! OL mempool database interface and its record types.

use serde::{Deserialize, Serialize};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::{OLBlockCommitment, OLTxId};

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Stored mempool transaction with ordering metadata.
///
/// Used by [`MempoolDatabase`] trait for storage and retrieval.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MempoolTxData {
    /// Transaction ID.
    txid: OLTxId,

    /// Raw transaction bytes.
    #[serde(with = "serde_bytes")]
    tx_bytes: Vec<u8>,

    /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
    ///
    /// Persists across restarts.
    timestamp_micros: u64,
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

    pub fn txid(&self) -> OLTxId {
        self.txid
    }

    pub fn tx_bytes(&self) -> &[u8] {
        &self.tx_bytes
    }

    pub fn timestamp_micros(&self) -> u64 {
        self.timestamp_micros
    }
}

/// Mempool writes that [`MempoolDatabase::apply_batch`] applies atomically.
///
/// A chain update collects every row it removes, every transaction it
/// restores, and the tip it moves to, so a crash or a failed write never
/// leaves part of an update behind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MempoolWriteBatch {
    deletes: Vec<OLTxId>,
    puts: Vec<MempoolTxData>,
    tip: Option<OLBlockCommitment>,
}

impl MempoolWriteBatch {
    /// Adds a transaction to delete.
    pub fn delete(&mut self, txid: OLTxId) {
        self.deletes.push(txid);
    }

    /// Adds a transaction to store.
    pub fn put(&mut self, data: MempoolTxData) {
        self.puts.push(data);
    }

    /// Records `tip` as the chain tip the stored transactions reflect.
    pub fn set_tip(&mut self, tip: OLBlockCommitment) {
        self.tip = Some(tip);
    }

    /// Returns the transactions to delete.
    pub fn deletes(&self) -> &[OLTxId] {
        &self.deletes
    }

    /// Returns the transactions to store.
    pub fn puts(&self) -> &[MempoolTxData] {
        &self.puts
    }

    /// Returns the tip to record, if any.
    pub fn tip(&self) -> Option<OLBlockCommitment> {
        self.tip
    }
}

/// Database interface for OL mempool transactions.
///
/// Stores transactions as opaque bytes with ordering metadata.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:mempool")
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

    /// Applies `batch` atomically: either every write lands or none does.
    ///
    /// Deletes apply before puts, so a transaction both deleted and stored
    /// ends up stored. Deleting a transaction the database lacks is not an
    /// error. The batch's tip, if set, replaces the recorded tip.
    fn apply_batch(&self, batch: MempoolWriteBatch) -> DbResult<()>;

    /// Returns the tip the last batch recorded, if any.
    fn get_tip(&self) -> DbResult<Option<OLBlockCommitment>>;
}
