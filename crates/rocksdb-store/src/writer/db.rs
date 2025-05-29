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
    pub const fn new(db: Arc<DB>, ops: DbOpsConfig) -> Self {
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
    use strata_db::traits::L1WriterDatabase;
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;
    use test;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    #[test]
    fn test_put_blob_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        seq_db.put_payload_entry(0, blob.clone()).unwrap();

        let stored_blob = seq_db.get_payload_entry_by_idx(0).unwrap();
        assert_eq!(stored_blob, Some(blob));
    }

    #[test]
    fn test_put_blob_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);
        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        seq_db.put_payload_entry(0, blob.clone()).unwrap();

        let result = seq_db.put_payload_entry(0, blob);

        // Should be ok to put to existing key
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);

        let entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        // Insert
        seq_db.put_payload_entry(0, entry.clone()).unwrap();

        let updated_entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        // Update existing idx
        seq_db.put_payload_entry(0, updated_entry.clone()).unwrap();
        let retrieved_entry = seq_db.get_payload_entry_by_idx(0).unwrap().unwrap();
        assert_eq!(updated_entry, retrieved_entry);
    }

    #[test]
    fn test_get_last_entry_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        let next_blob_idx = seq_db.get_next_payload_idx().unwrap();
        assert_eq!(
            next_blob_idx, 0,
            "There is no last blobidx in the beginning"
        );

        seq_db
            .put_payload_entry(next_blob_idx, blob.clone())
            .unwrap();
        // Now the next idx is 1

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        seq_db.put_payload_entry(1, blob.clone()).unwrap();
        let next_blob_idx = seq_db.get_next_payload_idx().unwrap();
        // Now the last idx is 2

        assert_eq!(next_blob_idx, 2);
    }

    // Intent related tests

    #[test]
    fn test_put_intent_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [0; 32].into();

        seq_db.put_intent_entry(intent_id, intent.clone()).unwrap();

        let stored_intent = seq_db.get_intent_by_id(intent_id).unwrap();
        assert_eq!(stored_intent, Some(intent));
    }

    #[test]
    fn test_put_intent_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBL1WriterDb::new(db, db_ops);
        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [0; 32].into();

        let result = seq_db.put_intent_entry(intent_id, intent.clone());
        assert!(result.is_ok());

        let retrieved = seq_db.get_intent_by_id(intent_id).unwrap().unwrap();
        assert_eq!(retrieved, intent);
    }
}
