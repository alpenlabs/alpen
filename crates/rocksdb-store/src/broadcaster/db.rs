use std::sync::Arc;

use rockbound::{
    utils::get_last, OptimisticTransactionDB as DB, SchemaDBOperationsExt, TransactionRetry,
};
use strata_db::{
    errors::DbError,
    traits::{self, L1BroadcastDatabase},
    types::L1TxEntry,
    DbResult,
};
use strata_primitives::buf::Buf32;

use super::schemas::{BcastL1TxIdSchema, BcastL1TxSchema};
use crate::{sequence::get_next_id, DbOpsConfig};

pub struct L1BroadcastDb {
    db: Arc<DB>,
    ops: DbOpsConfig,
}

impl L1BroadcastDb {
    pub fn new(db: Arc<DB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl L1BroadcastDatabase for L1BroadcastDb {
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>> {
        self.db
            .with_optimistic_txn(
                TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<Option<u64>, anyhow::Error> {
                    if txn.get::<BcastL1TxSchema>(&txid)?.is_none() {
                        let idx = get_next_id::<BcastL1TxIdSchema, DB>(txn)?;
                        txn.put::<BcastL1TxIdSchema>(&idx, &txid)?;
                        txn.put::<BcastL1TxSchema>(&txid, &txentry)?;
                        Ok(Some(idx))
                    } else {
                        txn.put::<BcastL1TxSchema>(&txid, &txentry)?;
                        Ok(None)
                    }
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()> {
        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |tx| {
                if let Some(id) = tx.get::<BcastL1TxIdSchema>(&idx)? {
                    Ok(tx.put::<BcastL1TxSchema>(&id, &txentry)?)
                } else {
                    Err(DbError::Other(format!(
                        "Entry does not exist for idx {idx:?}"
                    )))
                }
            })
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn del_tx_entry(&self, txid: Buf32) -> DbResult<bool> {
        let exists = self.db.get::<BcastL1TxSchema>(&txid)?.is_some();
        if exists {
            self.db.delete::<BcastL1TxSchema>(&txid)?;
        }
        Ok(exists)
    }

    fn del_tx_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = get_last::<BcastL1TxIdSchema>(self.db.as_ref())?.map(|(x, _)| x);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        let mut deleted_indices = Vec::new();

        // Use batch operations for efficiency
        self.db
            .with_optimistic_txn(
                TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    for idx in start_idx..=last_idx {
                        if let Some(txid) = txn.get::<BcastL1TxIdSchema>(&idx)? {
                            // Delete both the index mapping and the tx entry
                            txn.delete::<BcastL1TxIdSchema>(&idx)?;
                            txn.delete::<BcastL1TxSchema>(&txid)?;
                            deleted_indices.push(idx);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(deleted_indices)
    }

    fn get_tx_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        Ok(self.db.get::<BcastL1TxSchema>(&txid)?)
    }

    fn get_next_tx_idx(&self) -> DbResult<u64> {
        Ok(get_last::<BcastL1TxIdSchema>(self.db.as_ref())?
            .map(|(k, _)| k + 1)
            .unwrap_or_default())
    }

    fn get_txid(&self, idx: u64) -> DbResult<Option<Buf32>> {
        Ok(self.db.get::<BcastL1TxIdSchema>(&idx)?)
    }

    fn get_tx_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        if let Some(id) = self.get_txid(idx)? {
            Ok(self.db.get::<BcastL1TxSchema>(&id)?)
        } else {
            Err(DbError::Other(format!(
                "Entry does not exist for idx {idx:?}"
            )))
        }
    }

    fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>> {
        if let Some((_, txentry)) = get_last::<BcastL1TxSchema>(self.db.as_ref())? {
            Ok(Some(txentry))
        } else {
            Ok(None)
        }
    }
}

pub struct BroadcastDb {
    l1_broadcast_db: Arc<L1BroadcastDb>,
}

impl BroadcastDb {
    pub fn new(l1_broadcast_db: Arc<L1BroadcastDb>) -> Self {
        Self { l1_broadcast_db }
    }
}

impl traits::BroadcastDatabase for BroadcastDb {
    type L1BroadcastDB = L1BroadcastDb;

    fn l1_broadcast_db(&self) -> &Arc<Self::L1BroadcastDB> {
        &self.l1_broadcast_db
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use strata_db::{traits::L1BroadcastDatabase, types::L1TxStatus};
    use strata_primitives::buf::Buf32;
    use strata_test_utils::bitcoin::get_test_bitcoin_txs;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> L1BroadcastDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        L1BroadcastDb::new(db, db_ops)
    }

    fn generate_l1_tx_entry() -> (Buf32, L1TxEntry) {
        let txns = get_test_bitcoin_txs();
        let txid = txns[0].compute_txid().as_raw_hash().to_byte_array().into();
        let txentry = L1TxEntry::from_tx(&txns[0]);
        (txid, txentry)
    }
    #[test]
    fn test_get_last_tx_entry() {
        let db = setup_db();

        for _ in 0..2 {
            let (txid, txentry) = generate_l1_tx_entry();

            let _ = db.put_tx_entry(txid, txentry.clone()).unwrap();
            let last_entry = db.get_last_tx_entry().unwrap();

            assert_eq!(last_entry, Some(txentry));
        }
    }
    #[test]
    fn test_add_tx_new_entry() {
        let db = setup_db();

        let (txid, txentry) = generate_l1_tx_entry();

        let idx = db.put_tx_entry(txid, txentry.clone()).unwrap();

        assert_eq!(idx, Some(0));

        let stored_entry = db.get_tx_entry(idx.unwrap()).unwrap();
        assert_eq!(stored_entry, Some(txentry));
    }

    #[test]
    fn test_put_tx_existing_entry() {
        let broadcast_db = setup_db();

        let (txid, txentry) = generate_l1_tx_entry();

        let _ = broadcast_db.put_tx_entry(txid, txentry.clone()).unwrap();

        // Update the same txid
        let result = broadcast_db.put_tx_entry(txid, txentry);

        assert!(result.is_ok());
    }

    #[test]
    fn test_update_tx_entry() {
        let broadcast_db = setup_db();

        let (txid, txentry) = generate_l1_tx_entry();

        // Attempt to update non-existing index
        let result = broadcast_db.put_tx_entry_by_idx(0, txentry.clone());
        assert!(result.is_err());

        // Add and then update the entry by index
        let idx = broadcast_db.put_tx_entry(txid, txentry.clone()).unwrap();

        let mut updated_txentry = txentry;
        updated_txentry.status = L1TxStatus::Finalized { confirmations: 1 };

        broadcast_db
            .put_tx_entry_by_idx(idx.unwrap(), updated_txentry.clone())
            .unwrap();

        let stored_entry = broadcast_db.get_tx_entry(idx.unwrap()).unwrap();
        assert_eq!(stored_entry, Some(updated_txentry));
    }

    #[test]
    fn test_get_txentry_by_idx() {
        let broadcast_db = setup_db();

        // Test non-existing entry
        let result = broadcast_db.get_tx_entry(0);
        assert!(result.is_err());

        let (txid, txentry) = generate_l1_tx_entry();

        let idx = broadcast_db.put_tx_entry(txid, txentry.clone()).unwrap();

        let stored_entry = broadcast_db.get_tx_entry(idx.unwrap()).unwrap();
        assert_eq!(stored_entry, Some(txentry));
    }

    #[test]
    fn test_get_next_txidx() {
        let broadcast_db = setup_db();

        let next_txidx = broadcast_db.get_next_tx_idx().unwrap();
        assert_eq!(next_txidx, 0, "The next txidx is 0 in the beginning");

        let (txid, txentry) = generate_l1_tx_entry();

        let idx = broadcast_db.put_tx_entry(txid, txentry.clone()).unwrap();

        let next_txidx = broadcast_db.get_next_tx_idx().unwrap();

        assert_eq!(next_txidx, idx.unwrap() + 1);
    }

    #[test]
    fn test_del_tx_entry_single() {
        let broadcast_db = setup_db();
        let (txid, txentry) = generate_l1_tx_entry();

        // Insert tx entry
        broadcast_db
            .put_tx_entry(txid, txentry.clone())
            .expect("test: insert");

        // Verify it exists
        assert!(broadcast_db
            .get_tx_entry_by_id(txid)
            .expect("test: get")
            .is_some());

        // Delete it
        let deleted = broadcast_db.del_tx_entry(txid).expect("test: delete");
        assert!(
            deleted,
            "Should return true when deleting existing tx entry"
        );

        // Verify it's gone
        assert!(broadcast_db
            .get_tx_entry_by_id(txid)
            .expect("test: get after delete")
            .is_none());

        // Delete again should return false
        let deleted_again = broadcast_db.del_tx_entry(txid).expect("test: delete again");
        assert!(
            !deleted_again,
            "Should return false when deleting non-existent tx entry"
        );
    }

    #[test]
    fn test_del_tx_entries_from_idx() {
        let broadcast_db = setup_db();
        let txs = get_test_bitcoin_txs();

        // Generate different tx entries
        let txid1: Buf32 = txs[0].compute_txid().as_raw_hash().to_byte_array().into();
        let txid2: Buf32 = txs[1].compute_txid().as_raw_hash().to_byte_array().into();
        let txid3: Buf32 = txs[2].compute_txid().as_raw_hash().to_byte_array().into();
        let txid4: Buf32 = txs[3].compute_txid().as_raw_hash().to_byte_array().into();

        let txentry1 = L1TxEntry::from_tx(&txs[0]);
        let txentry2 = L1TxEntry::from_tx(&txs[1]);
        let txentry3 = L1TxEntry::from_tx(&txs[2]);
        let txentry4 = L1TxEntry::from_tx(&txs[3]);

        // Insert tx entries - they will get consecutive indices
        broadcast_db
            .put_tx_entry(txid1, txentry1)
            .expect("test: insert 1");
        broadcast_db
            .put_tx_entry(txid2, txentry2)
            .expect("test: insert 2");
        broadcast_db
            .put_tx_entry(txid3, txentry3)
            .expect("test: insert 3");
        broadcast_db
            .put_tx_entry(txid4, txentry4)
            .expect("test: insert 4");

        // Verify all exist by getting tx by idx
        assert!(broadcast_db
            .get_tx_entry(0)
            .expect("test: get idx 0")
            .is_some());
        assert!(broadcast_db
            .get_tx_entry(1)
            .expect("test: get idx 1")
            .is_some());
        assert!(broadcast_db
            .get_tx_entry(2)
            .expect("test: get idx 2")
            .is_some());
        assert!(broadcast_db
            .get_tx_entry(3)
            .expect("test: get idx 3")
            .is_some());

        // Delete from index 2 onwards
        let deleted_indices = broadcast_db
            .del_tx_entries_from_idx(2)
            .expect("test: delete from idx 2");
        assert_eq!(deleted_indices, vec![2, 3], "Should delete indices 2 and 3");

        // Verify indices 0 and 1 still exist, indices 2 and 3 are gone
        assert!(broadcast_db
            .get_tx_entry(0)
            .expect("test: get idx 0 after")
            .is_some());
        assert!(broadcast_db
            .get_tx_entry(1)
            .expect("test: get idx 1 after")
            .is_some());
        assert!(
            broadcast_db.get_tx_entry(2).is_err(),
            "Should error when getting deleted index 2"
        );
        assert!(
            broadcast_db.get_tx_entry(3).is_err(),
            "Should error when getting deleted index 3"
        );

        // Also verify the tx entries themselves are gone
        assert!(broadcast_db
            .get_tx_entry_by_id(txid3)
            .expect("test: get id 3")
            .is_none());
        assert!(broadcast_db
            .get_tx_entry_by_id(txid4)
            .expect("test: get id 4")
            .is_none());
    }

    #[test]
    fn test_del_tx_entries_empty_database() {
        let broadcast_db = setup_db();

        // Delete from empty database should return empty vec
        let deleted_indices = broadcast_db
            .del_tx_entries_from_idx(0)
            .expect("test: delete from empty");
        assert!(
            deleted_indices.is_empty(),
            "Should return empty vec for empty database"
        );
    }
}
