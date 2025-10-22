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
        if !exists {
            return Ok(false);
        }

        // Delete both the intent entry and its index mapping
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    // Find ALL index entries pointing to this ID by scanning IntentIdxSchema
                    // Note: IDs are not unique, multiple indices can point to the same ID
                    let mut iterator = txn.iter::<IntentIdxSchema>()?;
                    iterator.seek_to_first();

                    let mut indices_to_delete = Vec::new();
                    for item in iterator {
                        let (idx, intent_id) = item?.into_tuple();
                        if intent_id == id {
                            indices_to_delete.push(idx);
                        }
                    }

                    // Delete all index mappings found
                    for idx in indices_to_delete {
                        txn.delete::<IntentIdxSchema>(&idx)?;
                    }

                    // Delete the intent entry
                    txn.delete::<IntentSchema>(&id)?;
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(true)
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
    use strata_db::types::{BundledPayloadEntry, IntentEntry};
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    #[test]
    fn test_put_blob_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        writer_db.put_payload_entry(0, blob.clone()).unwrap();

        let stored_blob = writer_db.get_payload_entry_by_idx(0).unwrap();
        assert_eq!(stored_blob, Some(blob));
    }

    #[test]
    fn test_put_blob_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        writer_db.put_payload_entry(0, blob.clone()).unwrap();

        let result = writer_db.put_payload_entry(0, blob);

        // Should be ok to put to existing key
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        // Insert
        writer_db.put_payload_entry(0, entry.clone()).unwrap();

        let updated_entry: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        // Update existing idx
        writer_db
            .put_payload_entry(0, updated_entry.clone())
            .unwrap();
        let retrieved_entry = writer_db.get_payload_entry_by_idx(0).unwrap().unwrap();
        assert_eq!(updated_entry, retrieved_entry);
    }

    #[test]
    fn test_get_last_entry_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        let next_blob_idx = writer_db.get_next_payload_idx().unwrap();
        assert_eq!(
            next_blob_idx, 0,
            "There is no last blobidx in the beginning"
        );

        writer_db
            .put_payload_entry(next_blob_idx, blob.clone())
            .unwrap();
        // Now the next idx is 1

        let blob: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        writer_db.put_payload_entry(1, blob.clone()).unwrap();
        let next_blob_idx = writer_db.get_next_payload_idx().unwrap();
        // Now the last idx is 2

        assert_eq!(next_blob_idx, 2);
    }

    #[test]
    fn test_put_intent_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [0; 32].into();

        writer_db
            .put_intent_entry(intent_id, intent.clone())
            .unwrap();

        let stored_intent = writer_db.get_intent_by_id(intent_id).unwrap();
        assert_eq!(stored_intent, Some(intent));
    }

    #[test]
    fn test_put_intent_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [0; 32].into();

        let result = writer_db.put_intent_entry(intent_id, intent.clone());
        assert!(result.is_ok());

        let retrieved = writer_db.get_intent_by_id(intent_id).unwrap().unwrap();
        assert_eq!(retrieved, intent);
    }

    #[test]
    fn test_del_payload_entry_single() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let payload: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        let idx = 5;

        // Insert payload
        writer_db
            .put_payload_entry(idx, payload.clone())
            .expect("test: insert");

        // Verify it exists
        assert!(writer_db
            .get_payload_entry_by_idx(idx)
            .expect("test: get")
            .is_some());

        // Delete it
        let deleted = writer_db.del_payload_entry(idx).expect("test: delete");
        assert!(deleted, "Should return true when deleting existing payload");

        // Verify it's gone
        assert!(writer_db
            .get_payload_entry_by_idx(idx)
            .expect("test: get after delete")
            .is_none());

        // Delete again should return false
        let deleted_again = writer_db
            .del_payload_entry(idx)
            .expect("test: delete again");
        assert!(
            !deleted_again,
            "Should return false when deleting non-existent payload"
        );
    }

    #[test]
    fn test_del_payload_entries_from_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let payload: BundledPayloadEntry = ArbitraryGenerator::new().generate();

        // Insert payloads at indices 1, 3, 5, 7
        writer_db
            .put_payload_entry(1, payload.clone())
            .expect("test: insert 1");
        writer_db
            .put_payload_entry(3, payload.clone())
            .expect("test: insert 3");
        writer_db
            .put_payload_entry(5, payload.clone())
            .expect("test: insert 5");
        writer_db
            .put_payload_entry(7, payload.clone())
            .expect("test: insert 7");

        // Delete from index 4 onwards
        let deleted_indices = writer_db
            .del_payload_entries_from_idx(4)
            .expect("test: delete from idx 4");
        assert_eq!(deleted_indices, vec![5, 7], "Should delete indices 5 and 7");

        // Verify indices 1 and 3 still exist, indices 5 and 7 are gone
        assert!(writer_db
            .get_payload_entry_by_idx(1)
            .expect("test: get 1")
            .is_some());
        assert!(writer_db
            .get_payload_entry_by_idx(3)
            .expect("test: get 3")
            .is_some());
        assert!(writer_db
            .get_payload_entry_by_idx(5)
            .expect("test: get 5")
            .is_none());
        assert!(writer_db
            .get_payload_entry_by_idx(7)
            .expect("test: get 7")
            .is_none());

        // Delete from index 2 onwards
        let deleted_indices = writer_db
            .del_payload_entries_from_idx(2)
            .expect("test: delete from idx 2");
        assert_eq!(deleted_indices, vec![3], "Should delete index 3");

        // Verify only index 1 remains
        assert!(writer_db
            .get_payload_entry_by_idx(1)
            .expect("test: get 1 final")
            .is_some());
        assert!(writer_db
            .get_payload_entry_by_idx(3)
            .expect("test: get 3 final")
            .is_none());
    }

    #[test]
    fn test_del_intent_entry_single() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [1; 32].into();

        // Insert intent
        writer_db
            .put_intent_entry(intent_id, intent.clone())
            .expect("test: insert");

        // Verify it exists
        assert!(writer_db
            .get_intent_by_id(intent_id)
            .expect("test: get")
            .is_some());

        // Verify the index entry exists
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get by idx")
            .is_some());

        // Delete it
        let deleted = writer_db.del_intent_entry(intent_id).expect("test: delete");
        assert!(deleted, "Should return true when deleting existing intent");

        // Verify it's gone
        assert!(writer_db
            .get_intent_by_id(intent_id)
            .expect("test: get after delete")
            .is_none());

        // Verify the index entry is also gone
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get by idx after delete")
            .is_none());

        // Delete again should return false
        let deleted_again = writer_db
            .del_intent_entry(intent_id)
            .expect("test: delete again");
        assert!(
            !deleted_again,
            "Should return false when deleting non-existent intent"
        );
    }

    #[test]
    fn test_del_intent_entry_with_multiple_indices() {
        // This test verifies that del_intent_entry properly handles the case where
        // multiple index entries point to the same intent ID (non-unique IDs).
        // This can happen when the same checkpoint is reprocessed.
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();
        let intent_id: Buf32 = [7; 32].into();

        // Insert the same intent multiple times to create multiple index entries
        writer_db
            .put_intent_entry(intent_id, intent.clone())
            .expect("test: insert 1");
        writer_db
            .put_intent_entry(intent_id, intent.clone())
            .expect("test: insert 2");
        writer_db
            .put_intent_entry(intent_id, intent.clone())
            .expect("test: insert 3");

        // Verify all three indices exist and point to the same intent
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get idx 0")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(1)
            .expect("test: get idx 1")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(2)
            .expect("test: get idx 2")
            .is_some());

        // Verify the intent entry exists
        assert!(writer_db
            .get_intent_by_id(intent_id)
            .expect("test: get by id")
            .is_some());

        // Delete the intent - this should delete ALL index entries pointing to it
        let deleted = writer_db.del_intent_entry(intent_id).expect("test: delete");
        assert!(
            deleted,
            "Should return true when deleting existing intent with multiple indices"
        );

        // Verify the intent entry is gone
        assert!(writer_db
            .get_intent_by_id(intent_id)
            .expect("test: get by id after delete")
            .is_none());

        // Verify ALL index entries are gone
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get idx 0 after delete")
            .is_none());
        assert!(writer_db
            .get_intent_by_idx(1)
            .expect("test: get idx 1 after delete")
            .is_none());
        assert!(writer_db
            .get_intent_by_idx(2)
            .expect("test: get idx 2 after delete")
            .is_none());
    }

    #[test]
    fn test_del_intent_entries_from_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        let intent: IntentEntry = ArbitraryGenerator::new().generate();

        // Create different intent IDs
        let intent_id1: Buf32 = [1; 32].into();
        let intent_id2: Buf32 = [2; 32].into();
        let intent_id3: Buf32 = [3; 32].into();
        let intent_id4: Buf32 = [4; 32].into();

        // Insert intents - they will get consecutive indices
        writer_db
            .put_intent_entry(intent_id1, intent.clone())
            .expect("test: insert 1");
        writer_db
            .put_intent_entry(intent_id2, intent.clone())
            .expect("test: insert 2");
        writer_db
            .put_intent_entry(intent_id3, intent.clone())
            .expect("test: insert 3");
        writer_db
            .put_intent_entry(intent_id4, intent.clone())
            .expect("test: insert 4");

        // Verify all exist
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get idx 0")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(1)
            .expect("test: get idx 1")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(2)
            .expect("test: get idx 2")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(3)
            .expect("test: get idx 3")
            .is_some());

        // Delete from index 2 onwards
        let deleted_indices = writer_db
            .del_intent_entries_from_idx(2)
            .expect("test: delete from idx 2");
        assert_eq!(deleted_indices, vec![2, 3], "Should delete indices 2 and 3");

        // Verify indices 0 and 1 still exist, indices 2 and 3 are gone
        assert!(writer_db
            .get_intent_by_idx(0)
            .expect("test: get idx 0 after")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(1)
            .expect("test: get idx 1 after")
            .is_some());
        assert!(writer_db
            .get_intent_by_idx(2)
            .expect("test: get idx 2 after")
            .is_none());
        assert!(writer_db
            .get_intent_by_idx(3)
            .expect("test: get idx 3 after")
            .is_none());

        // Also verify the intent entries themselves are gone
        assert!(writer_db
            .get_intent_by_id(intent_id3)
            .expect("test: get id 3")
            .is_none());
        assert!(writer_db
            .get_intent_by_id(intent_id4)
            .expect("test: get id 4")
            .is_none());
    }

    #[test]
    fn test_del_payload_entries_empty_database() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        // Delete from empty database should return empty vec
        let deleted_indices = writer_db
            .del_payload_entries_from_idx(0)
            .expect("test: delete from empty");
        assert!(
            deleted_indices.is_empty(),
            "Should return empty vec for empty database"
        );
    }

    #[test]
    fn test_del_intent_entries_empty_database() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let writer_db = RBL1WriterDb::new(db, db_ops);

        // Delete from empty database should return empty vec
        let deleted_indices = writer_db
            .del_intent_entries_from_idx(0)
            .expect("test: delete from empty");
        assert!(
            deleted_indices.is_empty(),
            "Should return empty vec for empty database"
        );
    }
}
