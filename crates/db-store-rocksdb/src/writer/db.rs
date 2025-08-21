use std::sync::Arc;

use rockbound::{utils::get_last, OptimisticTransactionDB as DB, SchemaDBOperationsExt};
use strata_db::{
    errors::DbError,
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry},
    DbResult,
};
use strata_primitives::buf::Buf32;

use super::schemas::{IntentIdxSchema, IntentSchema, PayloadSchema};
use crate::{sequence::get_next_id, DbOpsConfig};

#[derive(Debug)]
pub struct RBL1WriterDb {
    db: Arc<DB>,
    ops: DbOpsConfig,
}

impl RBL1WriterDb {
    /// Wraps an existing database handle.
    ///
    /// Assumes it was opened with column families as defined in `STORE_COLUMN_FAMILIES`.
    // FIXME Make it better/generic.
    pub fn new(db: Arc<DB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl L1WriterDatabase for RBL1WriterDb {
    fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> DbResult<()> {
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |tx| -> Result<(), DbError> {
                    tx.put::<PayloadSchema>(&idx, &entry)?;
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_payload_entry_by_idx(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>> {
        Ok(self.db.get::<PayloadSchema>(&idx)?)
    }

    fn get_next_payload_idx(&self) -> DbResult<u64> {
        Ok(get_last::<PayloadSchema>(&*self.db)?
            .map(|(x, _)| x + 1)
            .unwrap_or(0))
    }

    fn del_payload_entry(&self, idx: u64) -> DbResult<bool> {
        let exists = self.db.get::<PayloadSchema>(&idx)?.is_some();
        if exists {
            self.db.delete::<PayloadSchema>(&idx)?;
        }
        Ok(exists)
    }

    fn del_payload_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = get_last::<PayloadSchema>(&*self.db)?.map(|(x, _)| x);
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
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    for idx in start_idx..=last_idx {
                        if txn.get::<PayloadSchema>(&idx)?.is_some() {
                            txn.delete::<PayloadSchema>(&idx)?;
                            deleted_indices.push(idx);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(deleted_indices)
    }

    fn put_intent_entry(&self, intent_id: Buf32, intent_entry: IntentEntry) -> DbResult<()> {
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |tx| -> Result<(), DbError> {
                    let idx = get_next_id::<IntentIdxSchema, DB>(tx)?;
                    tx.put::<IntentIdxSchema>(&idx, &intent_id)?;
                    tx.put::<IntentSchema>(&intent_id, &intent_entry)?;

                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_intent_by_id(&self, id: Buf32) -> DbResult<Option<IntentEntry>> {
        Ok(self.db.get::<IntentSchema>(&id)?)
    }

    fn get_intent_by_idx(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
        if let Some(id) = self.db.get::<IntentIdxSchema>(&idx)? {
            self.db
                .get::<IntentSchema>(&id)?
                .ok_or_else(|| {
                    DbError::Other(format!(
                    "Intent index({idx}) exists but corresponding id does not exist in writer db"
                ))
                })
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn get_next_intent_idx(&self) -> DbResult<u64> {
        Ok(get_last::<IntentIdxSchema>(&*self.db)?
            .map(|(x, _)| x + 1)
            .unwrap_or(0))
    }

    fn del_intent_entry(&self, id: Buf32) -> DbResult<bool> {
        let exists = self.db.get::<IntentSchema>(&id)?.is_some();
        if exists {
            self.db.delete::<IntentSchema>(&id)?;
        }
        Ok(exists)
    }

    fn del_intent_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = get_last::<IntentIdxSchema>(&*self.db)?.map(|(x, _)| x);
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
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    for idx in start_idx..=last_idx {
                        if let Some(intent_id) = txn.get::<IntentIdxSchema>(&idx)? {
                            // Delete both the index mapping and the intent entry
                            txn.delete::<IntentIdxSchema>(&idx)?;
                            txn.delete::<IntentSchema>(&intent_id)?;
                            deleted_indices.push(idx);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(deleted_indices)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_writer_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> RBL1WriterDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        RBL1WriterDb::new(db, db_ops)
    }

    l1_writer_db_tests!(setup_db());
}
