use std::{collections::HashMap, sync::Arc};

use strata_db_types::{traits::MempoolDatabase, types::MempoolTxMetadata, DbResult};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

/// Manager for mempool database operations
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct MempoolManager {
    db: Arc<dyn MempoolDatabase>,
}

impl MempoolManager {
    /// Create new instance of [`MempoolManager`]
    pub fn new(db: Arc<impl MempoolDatabase + 'static>) -> Self {
        Self { db }
    }

    /// Store a transaction in the mempool
    pub fn put_tx_entry(
        &self,
        txid: &OLTxId,
        tx: &OLTransaction,
        metadata: &MempoolTxMetadata,
    ) -> DbResult<()> {
        self.db.put_tx_entry(txid, tx, metadata)
    }

    /// Get a transaction from the mempool
    pub fn get_tx_entry(
        &self,
        txid: &OLTxId,
    ) -> DbResult<Option<(OLTransaction, MempoolTxMetadata)>> {
        self.db.get_tx_entry(txid)
    }

    /// Get multiple transactions from the mempool
    pub fn get_tx_entries(
        &self,
        txids: &[OLTxId],
    ) -> DbResult<HashMap<OLTxId, (OLTransaction, MempoolTxMetadata)>> {
        self.db.get_tx_entries(txids)
    }

    /// Delete a transaction from the mempool
    pub fn del_tx_entry(&self, txid: &OLTxId) -> DbResult<()> {
        self.db.del_tx_entry(txid)
    }

    /// Delete multiple transactions from the mempool
    pub fn del_tx_entries(&self, txids: &[OLTxId]) -> DbResult<()> {
        self.db.del_tx_entries(txids)
    }

    /// Get all transaction IDs in the mempool
    pub fn get_all_tx_ids(&self) -> DbResult<Vec<OLTxId>> {
        self.db.get_all_tx_ids()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountId;
    use strata_ol_chain_types_new::{TransactionExtra, TransactionPayload};

    use super::*;

    fn create_test_tx(seed: u8) -> (OLTxId, OLTransaction, MempoolTxMetadata) {
        let payload = TransactionPayload::GenericAccountMessage {
            target: AccountId::new([0u8; 32]),
            payload: vec![seed],
        };
        let extra = TransactionExtra::default();
        let tx = OLTransaction::new(payload, extra);
        let txid = tx.compute_txid();
        let metadata = MempoolTxMetadata {
            size_bytes: 100,
            entry_slot: seed as u64,
            entry_time: seed as u64,
        };
        (txid, tx, metadata)
    }

    #[test]
    fn test_mempool_manager_basic_operations() {
        use strata_db_store_sled::test_utils::get_test_sled_backend;
        use strata_db_types::traits::DatabaseBackend;

        let db = get_test_sled_backend();
        let manager = MempoolManager::new(db.mempool_db());

        let (txid1, tx1, metadata1) = create_test_tx(1);
        let (txid2, tx2, metadata2) = create_test_tx(2);

        // Test put and get
        manager.put_tx_entry(&txid1, &tx1, &metadata1).unwrap();
        let result = manager.get_tx_entry(&txid1).unwrap();
        assert!(result.is_some());
        let (retrieved_tx, retrieved_metadata) = result.unwrap();
        assert_eq!(retrieved_tx, tx1);
        assert_eq!(retrieved_metadata, metadata1);

        // Test batch get
        manager.put_tx_entry(&txid2, &tx2, &metadata2).unwrap();
        let batch_result = manager.get_tx_entries(&[txid1, txid2]).unwrap();
        assert_eq!(batch_result.len(), 2);

        // Test get all IDs
        let all_ids = manager.get_all_tx_ids().unwrap();
        assert_eq!(all_ids.len(), 2);
        assert!(all_ids.contains(&txid1));
        assert!(all_ids.contains(&txid2));

        // Test delete
        manager.del_tx_entry(&txid1).unwrap();
        let result = manager.get_tx_entry(&txid1).unwrap();
        assert!(result.is_none());

        // Test batch delete
        manager.del_tx_entries(&[txid2]).unwrap();
        let all_ids = manager.get_all_tx_ids().unwrap();
        assert_eq!(all_ids.len(), 0);
    }
}
