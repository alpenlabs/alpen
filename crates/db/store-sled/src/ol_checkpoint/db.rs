use strata_checkpoint_types::EpochSummary;
use strata_db_types::{
    DbError, DbResult,
    traits::OLCheckpointDatabase,
    types::{OLCheckpointEntry, OLCheckpointStatus},
};
use strata_identifiers::{Epoch, EpochCommitment};

use super::schemas::*;
use crate::{define_sled_database, utils::first};

define_sled_database!(
    pub struct OLCheckpointDBSled {
        checkpoint_tree: OLCheckpointSchema,
        unsigned_tree: UnsignedCheckpointIndexSchema,
        epoch_summary_tree: OLEpochSummarySchema,
    }
);

impl OLCheckpointDatabase for OLCheckpointDBSled {
    fn insert_epoch_summary(&self, summary: EpochSummary) -> DbResult<()> {
        let epoch_idx = summary.epoch() as u64;
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
        let Some(mut summaries) = self.epoch_summary_tree.get(&(epoch.epoch() as u64))? else {
            return Ok(None);
        };

        let terminal = epoch.to_block_commitment();
        let Ok(pos) = summaries.binary_search_by_key(&terminal, |s| *s.terminal()) else {
            return Ok(None);
        };

        Ok(Some(summaries.remove(pos)))
    }

    fn get_epoch_commitments_at(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>> {
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

    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let epoch_idx = epoch.epoch() as u64;
        let terminal = epoch.to_block_commitment();

        let Some(mut summaries) = self.epoch_summary_tree.get(&epoch_idx)? else {
            return Ok(false);
        };
        let old_summaries = summaries.clone();

        let Ok(pos) = summaries.binary_search_by_key(&terminal, |s| *s.terminal()) else {
            return Ok(false);
        };

        summaries.remove(pos);

        if summaries.is_empty() {
            self.epoch_summary_tree
                .compare_and_swap(epoch_idx, Some(old_summaries), None)?;
        } else {
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

    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> DbResult<()> {
        let is_unsigned = entry.status == OLCheckpointStatus::Unsigned;

        self.config
            .with_retry((&self.checkpoint_tree, &self.unsigned_tree), |(ct, ut)| {
                ct.insert(&epoch, &entry)?;

                if is_unsigned {
                    ut.insert(&epoch, &())?;
                } else {
                    ut.remove(&epoch)?;
                }

                Ok(())
            })?;

        Ok(())
    }

    fn get_checkpoint(&self, epoch: Epoch) -> DbResult<Option<OLCheckpointEntry>> {
        Ok(self.checkpoint_tree.get(&epoch)?)
    }

    fn get_last_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        Ok(self.checkpoint_tree.last()?.map(first))
    }

    fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        let mut iter = self.unsigned_tree.iter();
        Ok(iter.next().transpose()?.map(first))
    }

    fn del_checkpoint(&self, epoch: Epoch) -> DbResult<bool> {
        self.config
            .with_retry((&self.checkpoint_tree, &self.unsigned_tree), |(ct, ut)| {
                let existing = ct.get(&epoch)?;
                if existing.is_some() {
                    ct.remove(&epoch)?;
                    ut.remove(&epoch)?;
                    return Ok(true);
                }
                Ok(false)
            })
    }

    fn del_checkpoints_from_epoch(&self, start_epoch: Epoch) -> DbResult<Vec<Epoch>> {
        let mut keys = Vec::new();
        for item in self.checkpoint_tree.range(start_epoch..)? {
            let (epoch, _entry) = item?;
            keys.push(epoch);
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let deleted_epochs =
            self.config
                .with_retry((&self.checkpoint_tree, &self.unsigned_tree), |(ct, ut)| {
                    let mut deleted_epochs = Vec::new();
                    for epoch in &keys {
                        if ct.contains_key(epoch)? {
                            ct.remove(epoch)?;
                            ut.remove(epoch)?;
                            deleted_epochs.push(*epoch);
                        }
                    }
                    Ok(deleted_epochs)
                })?;
        Ok(deleted_epochs)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::ol_checkpoint_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLCheckpointDBSled, ol_checkpoint_db_tests);
}
