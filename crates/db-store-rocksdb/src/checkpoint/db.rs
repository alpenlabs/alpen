use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_checkpoint_types::EpochSummary;
use strata_db::{traits::CheckpointDatabase, types::CheckpointEntry, DbError, DbResult};
use strata_primitives::epoch::EpochCommitment;

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
    use strata_db_tests::checkpoint_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> RBCheckpointDB {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        RBCheckpointDB::new(db, db_ops)
    }

    checkpoint_db_tests!(setup_db());
}
