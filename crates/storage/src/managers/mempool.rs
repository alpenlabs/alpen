use std::{collections::HashMap, sync::Arc};

use strata_db_types::{traits::MempoolDatabase, DbResult};
use strata_identifiers::OLTxId;

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

    /// Store a raw transaction blob in the mempool
    pub fn put_tx_entry(&self, txid: &OLTxId, blob: &[u8]) -> DbResult<()> {
        self.db.put_tx_entry(txid, blob)
    }

    /// Get a raw transaction blob from the mempool
    pub fn get_tx_entry(&self, txid: &OLTxId) -> DbResult<Option<Vec<u8>>> {
        self.db.get_tx_entry(txid)
    }

    /// Get multiple transaction blobs from the mempool
    pub fn get_tx_entries(&self, txids: &[OLTxId]) -> DbResult<HashMap<OLTxId, Vec<u8>>> {
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
    use strata_acct_types::{AccountId, VarVec};
    use strata_codec::encode_to_vec;
    use strata_ol_chain_types_new::{
        GamTxPayload, OLTransaction, TransactionAttachment, TransactionPayload,
    };

    use super::*;

    fn create_test_tx_blob(seed: u8) -> (OLTxId, Vec<u8>) {
        let payload = GamTxPayload::new(
            AccountId::new([0u8; 32]),
            VarVec::from_vec(vec![seed]).unwrap(),
        );
        let tx = OLTransaction::new(
            TransactionAttachment::new_empty(),
            TransactionPayload::GenericAccountMessage(payload),
        );
        let txid = tx.compute_txid();
        let blob = encode_to_vec(&tx).unwrap();
        (txid, blob)
    }

    #[test]
    fn test_mempool_manager_basic_operations() {
        use strata_db_store_sled::test_utils::get_test_sled_backend;
        use strata_db_types::traits::DatabaseBackend;

        let db = get_test_sled_backend();
        let manager = MempoolManager::new(db.mempool_db());

        let (txid1, blob1) = create_test_tx_blob(1);
        let (txid2, blob2) = create_test_tx_blob(2);

        // Test put and get
        manager.put_tx_entry(&txid1, &blob1).unwrap();
        let result = manager.get_tx_entry(&txid1).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), blob1);

        // Test batch get
        manager.put_tx_entry(&txid2, &blob2).unwrap();
        let batch_result = manager.get_tx_entries(&[txid1, txid2]).unwrap();
        assert_eq!(batch_result.len(), 2);
        assert_eq!(batch_result.get(&txid1), Some(&blob1));
        assert_eq!(batch_result.get(&txid2), Some(&blob2));

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
