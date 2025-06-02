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

#[derive(Debug)]
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

#[derive(Debug)]
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
}
