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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::DatabaseBackend;
    use strata_identifiers::{Buf32, OLTxId};
    use threadpool::ThreadPool;

    use super::*;

    fn setup_manager() -> MempoolDbManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let mempool_db = db.mempool_db();
        MempoolDbManager::new(pool, mempool_db)
    }

    #[test]
    fn test_put_and_get_tx() {
        let manager = setup_manager();
        let txid = OLTxId::from(Buf32::from([1u8; 32]));
        let tx_bytes = vec![1, 2, 3, 4, 5];
        let timestamp_micros = 1_000_000;

        // Put transaction
        let data = MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros);
        manager.put_tx(data).expect("put_tx failed");

        // Get transaction
        let result = manager.get_tx(txid).expect("get_tx failed");
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.tx_bytes, tx_bytes);
        assert_eq!(retrieved.timestamp_micros, timestamp_micros);
    }

    #[test]
    fn test_get_nonexistent_tx() {
        let manager = setup_manager();
        let txid = OLTxId::from(Buf32::from([99u8; 32]));

        let result = manager.get_tx(txid).expect("get_tx failed");
        assert!(result.is_none());
    }

    #[test]
    fn test_del_tx() {
        let manager = setup_manager();
        let txid = OLTxId::from(Buf32::from([2u8; 32]));
        let tx_bytes = vec![6, 7, 8];
        let timestamp_micros = 1_000_000;

        // Put transaction
        let data = MempoolTxData::new(txid, tx_bytes.clone(), timestamp_micros);
        manager.put_tx(data).expect("put_tx failed");

        // Verify it exists
        assert!(manager.get_tx(txid).unwrap().is_some());

        // Delete transaction
        let existed = manager.del_tx(txid).expect("del_tx failed");
        assert!(existed);

        // Verify it's gone
        assert!(manager.get_tx(txid).unwrap().is_none());

        // Delete again should return false
        let existed = manager.del_tx(txid).expect("del_tx failed");
        assert!(!existed);
    }

    #[test]
    fn test_get_all_txs() {
        let manager = setup_manager();

        // Initially empty
        let all_txs = manager.get_all_txs().expect("get_all_txs failed");
        assert_eq!(all_txs.len(), 0);

        // Add multiple transactions
        let tx1_id = OLTxId::from(Buf32::from([10u8; 32]));
        let tx1_bytes = vec![1, 2, 3];
        let tx1_timestamp_micros = 1_000_000;

        let tx2_id = OLTxId::from(Buf32::from([20u8; 32]));
        let tx2_bytes = vec![4, 5, 6];
        let tx2_timestamp_micros = 2_000_000;

        manager
            .put_tx(MempoolTxData::new(
                tx1_id,
                tx1_bytes.clone(),
                tx1_timestamp_micros,
            ))
            .expect("put_tx failed");
        manager
            .put_tx(MempoolTxData::new(
                tx2_id,
                tx2_bytes.clone(),
                tx2_timestamp_micros,
            ))
            .expect("put_tx failed");

        // Get all transactions
        let all_txs = manager.get_all_txs().expect("get_all_txs failed");
        assert_eq!(all_txs.len(), 2);

        // Verify both transactions are present
        let tx1_found = all_txs.iter().any(|tx| {
            tx.txid == tx1_id
                && tx.tx_bytes == tx1_bytes
                && tx.timestamp_micros == tx1_timestamp_micros
        });
        let tx2_found = all_txs.iter().any(|tx| {
            tx.txid == tx2_id
                && tx.tx_bytes == tx2_bytes
                && tx.timestamp_micros == tx2_timestamp_micros
        });

        assert!(tx1_found);
        assert!(tx2_found);
    }
}
