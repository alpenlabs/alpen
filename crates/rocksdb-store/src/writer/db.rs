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
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_writer_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> RBL1WriterDb {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        RBL1WriterDb::new(db, db_ops)
    }

    #[test]
    fn test_put_blob_new_entry() {
        let db = setup_db();
        l1_writer_tests::test_put_blob_new_entry(&db);
    }

    #[test]
    fn test_put_blob_existing_entry() {
        let db = setup_db();
        l1_writer_tests::test_put_blob_existing_entry(&db);
    }

    #[test]
    fn test_update_entry() {
        let db = setup_db();
        l1_writer_tests::test_update_entry(&db);
    }

    #[test]
    fn test_get_last_entry_idx() {
        let db = setup_db();
        l1_writer_tests::test_get_last_entry_idx(&db);
    }

    #[test]
    fn test_put_intent_new_entry() {
        let db = setup_db();
        l1_writer_tests::test_put_intent_new_entry(&db);
    }

    #[test]
    fn test_put_intent_entry() {
        let db = setup_db();
        l1_writer_tests::test_put_intent_entry(&db);
    }
}
