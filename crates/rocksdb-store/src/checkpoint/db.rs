use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{
    traits::{CheckpointProvider, CheckpointStore},
    types::CheckpointEntry,
    DbError, DbResult,
};

use super::schemas::{BatchCheckpointIndexedSchema, BatchCheckpointSchema};
use crate::DbOpsConfig;

pub struct RBCheckpointDB {
    db: Arc<OptimisticTransactionDB>,
    #[allow(dead_code)]
    ops: DbOpsConfig,
}

impl RBCheckpointDB {
    /// Wraps an existing database handle.
    ///
    /// Assumes it was opened with column families as defined in `STORE_COLUMN_FAMILIES`.
    // FIXME Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }

    fn get_batch_checkpoint_old(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>> {
        Ok(self.db.get::<BatchCheckpointSchema>(&batchidx)?)
    }
}

impl CheckpointStore for RBCheckpointDB {
    fn put_batch_checkpoint(
        &self,
        batchidx: u64,
        batch_checkpoint: CheckpointEntry,
    ) -> DbResult<()> {
        Ok(self
            .db
            .put::<BatchCheckpointIndexedSchema>(&batchidx, &batch_checkpoint)?)
    }

    fn migrate_checkpoint_data(&self) -> DbResult<(u64, u64)> {
        // only check for sequential entries
        let mut batchidx = 0;
        let mut migrated_count = 0;
        while let Some(checkpoint) = self.get_batch_checkpoint_old(batchidx)? {
            if self.get_batch_checkpoint(batchidx)?.is_none() {
                self.put_batch_checkpoint(batchidx, checkpoint)?;
                migrated_count += 1;
            }

            batchidx += 1;
        }

        // check migration is completed
        // if old schema has data, ensure new schema is caught up to it
        if let Some(last_batch_idx_old) = batchidx.checked_sub(1) {
            match self.get_last_batch_idx()? {
                Some(last_batch_idx) if last_batch_idx >= last_batch_idx_old => {
                    // OK
                }
                Some(last_batch_idx) => {
                    return Err(DbError::CheckpointMigrationError(
                        last_batch_idx_old,
                        Some(last_batch_idx),
                    ));
                }
                None => {
                    return Err(DbError::CheckpointMigrationError(last_batch_idx_old, None));
                }
            }
        }
        Ok((batchidx, migrated_count))
    }
}

impl CheckpointProvider for RBCheckpointDB {
    fn get_batch_checkpoint(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>> {
        Ok(self.db.get::<BatchCheckpointIndexedSchema>(&batchidx)?)
    }

    fn get_last_batch_idx(&self) -> DbResult<Option<u64>> {
        Ok(rockbound::utils::get_last::<BatchCheckpointIndexedSchema>(&*self.db)?.map(|(x, _)| x))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;
    use test;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    #[test]
    fn test_batch_checkpoint_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let batchidx = 1;
        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_checkpoint(batchidx, checkpoint.clone())
            .unwrap();

        let retrieved_batch = seq_db.get_batch_checkpoint(batchidx).unwrap().unwrap();
        assert_eq!(checkpoint, retrieved_batch);
    }

    #[test]
    fn test_batch_checkpoint_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let batchidx = 1;
        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_checkpoint(batchidx, checkpoint.clone())
            .unwrap();

        seq_db
            .put_batch_checkpoint(batchidx, checkpoint.clone())
            .unwrap();
    }

    #[test]
    fn test_batch_checkpoint_non_monotonic_entries() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_checkpoint(100, checkpoint.clone())
            .unwrap();
        seq_db.put_batch_checkpoint(1, checkpoint.clone()).unwrap();
        seq_db.put_batch_checkpoint(3, checkpoint.clone()).unwrap();
    }

    #[test]
    fn test_get_last_batch_checkpoint_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db
            .put_batch_checkpoint(100, checkpoint.clone())
            .unwrap();
        seq_db.put_batch_checkpoint(1, checkpoint.clone()).unwrap();
        seq_db.put_batch_checkpoint(3, checkpoint.clone()).unwrap();

        let last_idx = seq_db.get_last_batch_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);

        seq_db.put_batch_checkpoint(50, checkpoint.clone()).unwrap();
        let last_idx = seq_db.get_last_batch_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);
    }
}
