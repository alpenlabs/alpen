use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{traits::CheckpointDatabase, types::CheckpointEntry, DbError, DbResult};
use strata_primitives::epoch::EpochCommitment;
use strata_state::batch::EpochSummary;

use super::schemas::*;
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct RBCheckpointDB {
    db: Arc<OptimisticTransactionDB>,
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
}

impl CheckpointDatabase for RBCheckpointDB {
    fn insert_epoch_summary(&self, summary: EpochSummary) -> DbResult<()> {
        let epoch_idx = summary.epoch();
        let commitment = summary.get_epoch_commitment();
        let terminal = summary.terminal();

        // This is kinda nontrivial so we don't want concurrent writes to
        // clobber each other, so we do it in a transaction.
        //
        // That would probably never happen, but better safe than sorry!
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| {
                    let mut summaries: Vec<EpochSummary> = txn
                        .get_for_update::<EpochSummarySchema>(&epoch_idx)?
                        .unwrap_or_else(Vec::new);

                    // Find where the summary should go, or return error if it's
                    // already there.
                    let pos = match summaries.binary_search_by_key(&terminal, |s| s.terminal()) {
                        Ok(_) => return Err(DbError::OverwriteEpoch(commitment))?,
                        Err(p) => p,
                    };

                    // Insert the summary into the list where it goes and put it
                    // back in the database.
                    summaries.insert(pos, summary);
                    txn.put::<EpochSummarySchema>(&epoch_idx, &summaries)?;

                    Ok::<_, anyhow::Error>(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        let Some(mut summaries) = self.db.get::<EpochSummarySchema>(&epoch.epoch())? else {
            return Ok(None);
        };

        // Binary search over the summaries to find the one we're looking for.
        let terminal = epoch.to_block_commitment();
        let Ok(pos) = summaries.binary_search_by_key(&terminal, |s| *s.terminal()) else {
            return Ok(None);
        };

        Ok(Some(summaries.remove(pos)))
    }

    fn get_epoch_commitments_at(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>> {
        // Okay looking at this now, this clever design seems pretty inefficient now.
        let summaries = self
            .db
            .get::<EpochSummarySchema>(&epoch)?
            .unwrap_or_else(Vec::new);
        Ok(summaries
            .into_iter()
            .map(|s| s.get_epoch_commitment())
            .collect::<Vec<_>>())
    }

    fn get_last_summarized_epoch(&self) -> DbResult<Option<u64>> {
        Ok(rockbound::utils::get_last::<EpochSummarySchema>(&*self.db)?.map(|(x, _)| x))
    }

    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let epoch_idx = epoch.epoch();
        let terminal = epoch.to_block_commitment();

        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<bool, anyhow::Error> {
                    let Some(mut summaries) =
                        txn.get_for_update::<EpochSummarySchema>(&epoch_idx)?
                    else {
                        return Ok(false);
                    };

                    // Find the summary to delete
                    let Ok(pos) = summaries.binary_search_by_key(&terminal, |s| *s.terminal())
                    else {
                        return Ok(false);
                    };

                    // Remove the summary from the vector
                    summaries.remove(pos);

                    // If vector is now empty, delete the entire entry
                    if summaries.is_empty() {
                        txn.delete::<EpochSummarySchema>(&epoch_idx)?;
                    } else {
                        // Otherwise, update with the remaining summaries
                        txn.put::<EpochSummarySchema>(&epoch_idx, &summaries)?;
                    }

                    Ok(true)
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn del_epoch_summaries_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>> {
        let last_epoch = self.get_last_summarized_epoch()?;
        let Some(last_epoch) = last_epoch else {
            return Ok(Vec::new());
        };

        if start_epoch > last_epoch {
            return Ok(Vec::new());
        }

        let mut deleted_epochs = Vec::new();

        // Use batch operations for efficiency
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    for epoch in start_epoch..=last_epoch {
                        if txn.get::<EpochSummarySchema>(&epoch)?.is_some() {
                            txn.delete::<EpochSummarySchema>(&epoch)?;
                            deleted_epochs.push(epoch);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(deleted_epochs)
    }

    fn put_checkpoint(&self, epoch: u64, entry: CheckpointEntry) -> DbResult<()> {
        Ok(self.db.put::<CheckpointSchema>(&epoch, &entry)?)
    }

    fn get_checkpoint(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>> {
        Ok(self.db.get::<CheckpointSchema>(&batchidx)?)
    }

    fn get_last_checkpoint_idx(&self) -> DbResult<Option<u64>> {
        Ok(rockbound::utils::get_last::<CheckpointSchema>(&*self.db)?.map(|(x, _)| x))
    }

    fn del_checkpoint(&self, epoch: u64) -> DbResult<bool> {
        let exists = self.db.get::<CheckpointSchema>(&epoch)?.is_some();
        if exists {
            self.db.delete::<CheckpointSchema>(&epoch)?;
        }
        Ok(exists)
    }

    fn del_checkpoints_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>> {
        let last_epoch = self.get_last_checkpoint_idx()?;
        let Some(last_epoch) = last_epoch else {
            return Ok(Vec::new());
        };

        if start_epoch > last_epoch {
            return Ok(Vec::new());
        }

        let mut deleted_epochs = Vec::new();

        // Use batch operations for efficiency
        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| -> Result<(), anyhow::Error> {
                    for epoch in start_epoch..=last_epoch {
                        if txn.get::<CheckpointSchema>(&epoch)?.is_some() {
                            txn.delete::<CheckpointSchema>(&epoch)?;
                            deleted_epochs.push(epoch);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))?;

        Ok(deleted_epochs)
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
    fn test_insert_summary_single() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let summary: EpochSummary = ArbitraryGenerator::new().generate();
        let commitment = summary.get_epoch_commitment();
        seq_db.insert_epoch_summary(summary).expect("test: insert");

        let stored = seq_db
            .get_epoch_summary(commitment)
            .expect("test: get")
            .expect("test: get missing");
        assert_eq!(stored, summary);

        let commitments = seq_db
            .get_epoch_commitments_at(commitment.epoch())
            .expect("test: get at epoch");

        assert_eq!(commitments.as_slice(), &[commitment]);
    }

    #[test]
    fn test_insert_summary_overwrite() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let summary: EpochSummary = ArbitraryGenerator::new().generate();
        seq_db.insert_epoch_summary(summary).expect("test: insert");
        seq_db
            .insert_epoch_summary(summary)
            .expect_err("test: passed unexpectedly");
    }

    #[test]
    fn test_insert_summary_multiple() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let mut ag = ArbitraryGenerator::new();
        let summary1: EpochSummary = ag.generate();
        let epoch = summary1.epoch();
        let summary2 = EpochSummary::new(
            epoch,
            ag.generate(),
            ag.generate(),
            ag.generate(),
            ag.generate(),
        );

        let commitment1 = summary1.get_epoch_commitment();
        let commitment2 = summary2.get_epoch_commitment();
        seq_db.insert_epoch_summary(summary1).expect("test: insert");
        seq_db.insert_epoch_summary(summary2).expect("test: insert");

        let stored1 = seq_db
            .get_epoch_summary(commitment1)
            .expect("test: get")
            .expect("test: get missing");
        assert_eq!(stored1, summary1);

        let stored2 = seq_db
            .get_epoch_summary(commitment2)
            .expect("test: get")
            .expect("test: get missing");
        assert_eq!(stored2, summary2);

        let mut commitments = vec![commitment1, commitment2];
        commitments.sort();

        let mut stored_commitments = seq_db
            .get_epoch_commitments_at(epoch)
            .expect("test: get at epoch");
        stored_commitments.sort();

        assert_eq!(stored_commitments, commitments);
    }

    #[test]
    fn test_batch_checkpoint_new_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let batchidx = 1;
        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();

        let retrieved_batch = seq_db.get_checkpoint(batchidx).unwrap().unwrap();
        assert_eq!(checkpoint, retrieved_batch);
    }

    #[test]
    fn test_batch_checkpoint_existing_entry() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let batchidx = 1;
        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();
        seq_db.put_checkpoint(batchidx, checkpoint.clone()).unwrap();
    }

    #[test]
    fn test_batch_checkpoint_non_monotonic_entries() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db.put_checkpoint(100, checkpoint.clone()).unwrap();
        seq_db.put_checkpoint(1, checkpoint.clone()).unwrap();
        seq_db.put_checkpoint(3, checkpoint.clone()).unwrap();
    }

    #[test]
    fn test_get_last_batch_checkpoint_idx() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        seq_db.put_checkpoint(100, checkpoint.clone()).unwrap();
        seq_db.put_checkpoint(1, checkpoint.clone()).unwrap();
        seq_db.put_checkpoint(3, checkpoint.clone()).unwrap();

        let last_idx = seq_db.get_last_checkpoint_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);

        seq_db.put_checkpoint(50, checkpoint.clone()).unwrap();
        let last_idx = seq_db.get_last_checkpoint_idx().unwrap().unwrap();
        assert_eq!(last_idx, 100);
    }

    /// Tests a peculiar issue with `default_codec` in rockbound schema. If it is used instead of
    /// `seek_key_codec`, the last_idx won't grow beyond 255.
    #[test]
    fn test_256_checkpoints() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let seq_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();

        for expected_idx in 0..=256 {
            let last_idx = seq_db.get_last_checkpoint_idx().unwrap().unwrap_or(0);
            assert_eq!(last_idx, expected_idx);

            // Insert one to db
            seq_db
                .put_checkpoint(last_idx + 1, checkpoint.clone())
                .unwrap();
        }
    }

    #[test]
    fn test_del_checkpoint_single() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();
        let epoch = 5;

        // Insert checkpoint
        chkpt_db
            .put_checkpoint(epoch, checkpoint.clone())
            .expect("test: insert");

        // Verify it exists
        assert!(chkpt_db.get_checkpoint(epoch).expect("test: get").is_some());

        // Delete it
        let deleted = chkpt_db.del_checkpoint(epoch).expect("test: delete");
        assert!(
            deleted,
            "Should return true when deleting existing checkpoint"
        );

        // Verify it's gone
        assert!(chkpt_db
            .get_checkpoint(epoch)
            .expect("test: get after delete")
            .is_none());

        // Delete again should return false
        let deleted_again = chkpt_db.del_checkpoint(epoch).expect("test: delete again");
        assert!(
            !deleted_again,
            "Should return false when deleting non-existent checkpoint"
        );
    }

    #[test]
    fn test_del_checkpoints_from_epoch() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        let checkpoint: CheckpointEntry = ArbitraryGenerator::new().generate();

        // Insert checkpoints for epochs 1, 3, 5, 7
        chkpt_db
            .put_checkpoint(1, checkpoint.clone())
            .expect("test: insert 1");
        chkpt_db
            .put_checkpoint(3, checkpoint.clone())
            .expect("test: insert 3");
        chkpt_db
            .put_checkpoint(5, checkpoint.clone())
            .expect("test: insert 5");
        chkpt_db
            .put_checkpoint(7, checkpoint.clone())
            .expect("test: insert 7");

        // Delete from epoch 4 onwards
        let deleted_epochs = chkpt_db
            .del_checkpoints_from_epoch(4)
            .expect("test: delete from epoch 4");
        assert_eq!(deleted_epochs, vec![5, 7], "Should delete epochs 5 and 7");

        // Verify epochs 1 and 3 still exist, epochs 5 and 7 are gone
        assert!(chkpt_db.get_checkpoint(1).expect("test: get 1").is_some());
        assert!(chkpt_db.get_checkpoint(3).expect("test: get 3").is_some());
        assert!(chkpt_db.get_checkpoint(5).expect("test: get 5").is_none());
        assert!(chkpt_db.get_checkpoint(7).expect("test: get 7").is_none());

        // Delete from epoch 2 onwards
        let deleted_epochs = chkpt_db
            .del_checkpoints_from_epoch(2)
            .expect("test: delete from epoch 2");
        assert_eq!(deleted_epochs, vec![3], "Should delete epoch 3");

        // Verify only epoch 1 remains
        assert!(chkpt_db
            .get_checkpoint(1)
            .expect("test: get 1 final")
            .is_some());
        assert!(chkpt_db
            .get_checkpoint(3)
            .expect("test: get 3 final")
            .is_none());
    }

    #[test]
    fn test_del_epoch_summary_single() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        let summary: EpochSummary = ArbitraryGenerator::new().generate();
        let commitment = summary.get_epoch_commitment();

        // Insert summary
        chkpt_db
            .insert_epoch_summary(summary)
            .expect("test: insert");

        // Verify it exists
        assert!(chkpt_db
            .get_epoch_summary(commitment)
            .expect("test: get")
            .is_some());

        // Delete it
        let deleted = chkpt_db
            .del_epoch_summary(commitment)
            .expect("test: delete");
        assert!(deleted, "Should return true when deleting existing summary");

        // Verify it's gone
        assert!(chkpt_db
            .get_epoch_summary(commitment)
            .expect("test: get after delete")
            .is_none());

        // Delete again should return false
        let deleted_again = chkpt_db
            .del_epoch_summary(commitment)
            .expect("test: delete again");
        assert!(
            !deleted_again,
            "Should return false when deleting non-existent summary"
        );
    }

    #[test]
    fn test_del_epoch_summaries_from_epoch() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        let mut ag = ArbitraryGenerator::new();

        // Create summaries for epochs 1, 2, 3
        let summary1: EpochSummary = EpochSummary::new(
            1,
            ag.generate(),
            ag.generate(),
            ag.generate(),
            ag.generate(),
        );
        let summary2: EpochSummary = EpochSummary::new(
            2,
            ag.generate(),
            ag.generate(),
            ag.generate(),
            ag.generate(),
        );
        let summary3: EpochSummary = EpochSummary::new(
            3,
            ag.generate(),
            ag.generate(),
            ag.generate(),
            ag.generate(),
        );

        let commitment1 = summary1.get_epoch_commitment();
        let commitment2 = summary2.get_epoch_commitment();
        let commitment3 = summary3.get_epoch_commitment();

        // Insert all summaries
        chkpt_db
            .insert_epoch_summary(summary1)
            .expect("test: insert 1");
        chkpt_db
            .insert_epoch_summary(summary2)
            .expect("test: insert 2");
        chkpt_db
            .insert_epoch_summary(summary3)
            .expect("test: insert 3");

        // Delete from epoch 2 onwards
        let deleted_epochs = chkpt_db
            .del_epoch_summaries_from_epoch(2)
            .expect("test: delete from epoch 2");
        assert_eq!(deleted_epochs, vec![2, 3], "Should delete epochs 2 and 3");

        // Verify epoch 1 still exists, epochs 2 and 3 are gone
        assert!(chkpt_db
            .get_epoch_summary(commitment1)
            .expect("test: get 1")
            .is_some());
        assert!(chkpt_db
            .get_epoch_summary(commitment2)
            .expect("test: get 2")
            .is_none());
        assert!(chkpt_db
            .get_epoch_summary(commitment3)
            .expect("test: get 3")
            .is_none());

        // Delete from epoch 0 onwards (should delete epoch 1)
        let deleted_epochs = chkpt_db
            .del_epoch_summaries_from_epoch(0)
            .expect("test: delete from epoch 0");
        assert_eq!(deleted_epochs, vec![1], "Should delete epoch 1");

        // Verify all are gone
        assert!(chkpt_db
            .get_epoch_summary(commitment1)
            .expect("test: get 1 final")
            .is_none());
    }

    #[test]
    fn test_del_epoch_summary_from_multiple() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        let mut ag = ArbitraryGenerator::new();
        let summary1: EpochSummary = ag.generate();
        let epoch = summary1.epoch();
        let summary2 = EpochSummary::new(
            epoch,
            ag.generate(),
            ag.generate(),
            ag.generate(),
            ag.generate(),
        );

        let commitment1 = summary1.get_epoch_commitment();
        let commitment2 = summary2.get_epoch_commitment();

        // Insert both summaries
        chkpt_db
            .insert_epoch_summary(summary1)
            .expect("test: insert 1");
        chkpt_db
            .insert_epoch_summary(summary2)
            .expect("test: insert 2");

        // Verify both exist
        assert!(chkpt_db
            .get_epoch_summary(commitment1)
            .expect("test: get 1")
            .is_some());
        assert!(chkpt_db
            .get_epoch_summary(commitment2)
            .expect("test: get 2")
            .is_some());

        // Delete first summary
        let deleted = chkpt_db
            .del_epoch_summary(commitment1)
            .expect("test: delete 1");
        assert!(deleted);

        // Verify first is gone, second still exists
        assert!(chkpt_db
            .get_epoch_summary(commitment1)
            .expect("test: get 1 after delete")
            .is_none());
        assert!(chkpt_db
            .get_epoch_summary(commitment2)
            .expect("test: get 2 after delete")
            .is_some());

        // Delete second summary
        let deleted = chkpt_db
            .del_epoch_summary(commitment2)
            .expect("test: delete 2");
        assert!(deleted);

        // Verify both are gone
        assert!(chkpt_db
            .get_epoch_summary(commitment1)
            .expect("test: get 1 final")
            .is_none());
        assert!(chkpt_db
            .get_epoch_summary(commitment2)
            .expect("test: get 2 final")
            .is_none());
    }

    #[test]
    fn test_del_checkpoints_empty_database() {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        let chkpt_db = RBCheckpointDB::new(db, db_ops);

        // Delete from empty database should return empty vec
        let deleted_epochs = chkpt_db
            .del_checkpoints_from_epoch(0)
            .expect("test: delete from empty");
        assert!(
            deleted_epochs.is_empty(),
            "Should return empty vec for empty database"
        );

        let deleted_epochs = chkpt_db
            .del_epoch_summaries_from_epoch(0)
            .expect("test: delete summaries from empty");
        assert!(
            deleted_epochs.is_empty(),
            "Should return empty vec for empty database"
        );
    }
}
