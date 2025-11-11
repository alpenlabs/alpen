use strata_checkpoint_types::EpochSummary;
use strata_db_types::{
    DbError, DbResult,
    traits::CheckpointDatabase,
    types::{CheckpointEntry, CheckpointProvingStatus},
};
use strata_primitives::epoch::EpochCommitment;

use super::schemas::*;
use crate::{define_sled_database, utils::first};

define_sled_database!(
    pub struct CheckpointDBSled {
        checkpoint_tree: CheckpointSchema,
        epoch_summary_tree: EpochSummarySchema,
        pending_proof_tree: PendingProofIndexSchema,
    }
);

impl CheckpointDatabase for CheckpointDBSled {
    fn insert_epoch_summary(&self, summary: EpochSummary) -> DbResult<()> {
        let epoch_idx = summary.epoch();
        let commitment = summary.get_epoch_commitment();
        let terminal = summary.terminal();

        let old_summaries = self.epoch_summary_tree.get(&epoch_idx)?;
        let mut summaries = old_summaries.clone().unwrap_or_default();
        let pos = match summaries.binary_search_by_key(&terminal, |s| s.terminal()) {
            Ok(_) => return Err(DbError::OverwriteEpoch(commitment)),
            Err(p) => p,
        };
        summaries.insert(pos, summary);
        self.epoch_summary_tree
            .compare_and_swap(epoch_idx, old_summaries, Some(summaries))?;
        Ok(())
    }

    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        let Some(mut summaries) = self.epoch_summary_tree.get(&epoch.epoch())? else {
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
            .epoch_summary_tree
            .get(&epoch)?
            .unwrap_or_else(Vec::new);
        Ok(summaries
            .into_iter()
            .map(|s| s.get_epoch_commitment())
            .collect::<Vec<_>>())
    }

    fn get_last_summarized_epoch(&self) -> DbResult<Option<u64>> {
        Ok(self.epoch_summary_tree.last()?.map(first))
    }

    fn put_checkpoint(&self, epoch: u64, entry: CheckpointEntry) -> DbResult<()> {
        let is_pending = entry.proving_status == CheckpointProvingStatus::PendingProof;

        self.config.with_retry(
            (&self.checkpoint_tree, &self.pending_proof_tree),
            |(ct, pt)| {
                ct.insert(&epoch, &entry)?;

                if is_pending {
                    pt.insert(&epoch, &())?;
                } else {
                    pt.remove(&epoch)?;
                }

                Ok(())
            },
        )?;

        Ok(())
    }

    fn get_checkpoint(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>> {
        Ok(self.checkpoint_tree.get(&batchidx)?)
    }

    fn get_last_checkpoint_idx(&self) -> DbResult<Option<u64>> {
        Ok(self.checkpoint_tree.last()?.map(first))
    }

    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let epoch_idx = epoch.epoch();
        let terminal = epoch.to_block_commitment();

        let Some(mut summaries) = self.epoch_summary_tree.get(&epoch_idx)? else {
            return Ok(false);
        };
        let old_summaries = summaries.clone(); // for CAS

        // Find the summary to delete
        let Ok(pos) = summaries.binary_search_by_key(&terminal, |s| *s.terminal()) else {
            return Ok(false);
        };

        // Remove the summary from the vector
        summaries.remove(pos);

        // If vector is now empty, delete the entire entry using CAS
        if summaries.is_empty() {
            self.epoch_summary_tree
                .compare_and_swap(epoch_idx, Some(old_summaries), None)?;
        } else {
            // Otherwise, update with the remaining summaries
            self.epoch_summary_tree.compare_and_swap(
                epoch_idx,
                Some(old_summaries),
                Some(summaries),
            )?;
        }

        Ok(true)
    }

    fn del_epoch_summaries_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>> {
        let last_epoch = self.get_last_summarized_epoch()?;
        let Some(last_epoch) = last_epoch else {
            return Ok(Vec::new());
        };

        if start_epoch > last_epoch {
            return Ok(Vec::new());
        }

        let deleted_epochs = self
            .config
            .with_retry((&self.epoch_summary_tree,), |(est,)| {
                let mut deleted_epochs = Vec::new();
                for epoch in start_epoch..=last_epoch {
                    if est.contains_key(&epoch)? {
                        est.remove(&epoch)?;
                        deleted_epochs.push(epoch);
                    }
                }
                Ok(deleted_epochs)
            })?;
        Ok(deleted_epochs)
    }

    fn del_checkpoint(&self, epoch: u64) -> DbResult<bool> {
        self.config.with_retry(
            (&self.checkpoint_tree, &self.pending_proof_tree),
            |(ct, pt)| {
                let existing = ct.get(&epoch)?;
                if existing.is_some() {
                    ct.remove(&epoch)?;
                    pt.remove(&epoch)?;
                    return Ok(true);
                }
                Ok(false)
            },
        )
    }

    fn del_checkpoints_from_epoch(&self, start_epoch: u64) -> DbResult<Vec<u64>> {
        let last_epoch = self.get_last_checkpoint_idx()?;
        let Some(last_epoch) = last_epoch else {
            return Ok(Vec::new());
        };

        if start_epoch > last_epoch {
            return Ok(Vec::new());
        }

        let deleted_epochs = self.config.with_retry(
            (&self.checkpoint_tree, &self.pending_proof_tree),
            |(ct, pt)| {
                let mut deleted_epochs = Vec::new();
                for epoch in start_epoch..=last_epoch {
                    if ct.contains_key(&epoch)? {
                        ct.remove(&epoch)?;
                        pt.remove(&epoch)?;
                        deleted_epochs.push(epoch);
                    }
                }
                Ok(deleted_epochs)
            },
        )?;
        Ok(deleted_epochs)
    }

    fn get_next_unproven_checkpoint_idx(&self) -> DbResult<Option<u64>> {
        let mut iter = self.pending_proof_tree.iter();
        Ok(iter.next().transpose()?.map(first))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::checkpoint_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(CheckpointDBSled, checkpoint_db_tests);
}
