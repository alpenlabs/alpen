use std::sync::Arc;

use ops::mempool::{Context, MempoolDataOps};
use strata_db_types::{traits::MempoolDatabase, types::MempoolTxData, DbResult};
use strata_identifiers::OLTxId;
use threadpool::ThreadPool;

use crate::ops;

/// Database manager for mempool transaction persistence.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't have Debug implementation"
)]
pub struct MempoolDbManager {
    ops: MempoolDataOps,
}

impl MempoolDbManager {
    /// Create new instance of [`MempoolDbManager`].
    pub fn new(pool: ThreadPool, db: Arc<impl MempoolDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Store a transaction in the mempool database.
    pub fn put_tx(&self, data: MempoolTxData) -> DbResult<()> {
        self.ops.put_tx_blocking(data)
    }

    /// Retrieve a transaction from the mempool database.
    pub fn get_tx(&self, txid: OLTxId) -> DbResult<Option<MempoolTxData>> {
        self.ops.get_tx_blocking(txid)
    }

    /// Retrieve all transactions from the mempool database.
    pub fn get_all_txs(&self) -> DbResult<Vec<MempoolTxData>> {
        self.ops.get_all_txs_blocking()
    }

    /// Delete a transaction from the mempool database.
    ///
    /// Returns `true` if the transaction existed, `false` otherwise.
    pub fn del_tx(&self, txid: OLTxId) -> DbResult<bool> {
        self.ops.del_tx_blocking(txid)
    }
}
