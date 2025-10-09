use std::sync::Arc;

use rockbound::{
    utils::get_last, OptimisticTransactionDB as DB, SchemaDBOperationsExt, TransactionRetry,
};
use strata_db::{errors::DbError, traits::L1BroadcastDatabase, types::L1TxEntry, DbResult};
use strata_identifiers::Buf32;

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

#[cfg(test)]
mod tests {
    use strata_db_tests::l1_broadcast_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> L1BroadcastDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        L1BroadcastDb::new(db, db_ops)
    }

    l1_broadcast_db_tests!(setup_db());
}
