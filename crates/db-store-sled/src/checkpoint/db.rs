use std::sync::Arc;

use strata_db::{DbError, DbResult, traits::CheckpointDatabase, types::CheckpointEntry};
use strata_primitives::epoch::EpochCommitment;
use strata_state::batch::EpochSummary;
use typed_sled::{SledDb, SledTree};

use super::schemas::*;
use crate::{SledDbConfig, utils::first};

#[derive(Debug)]
pub struct CheckpointDBSled {
    checkpoint_tree: SledTree<CheckpointSchema>,
    epoch_summary_tree: SledTree<EpochSummarySchema>,
    _config: SledDbConfig,
}

impl CheckpointDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            checkpoint_tree: db.get_tree()?,
            epoch_summary_tree: db.get_tree()?,
            _config: config,
        })
    }
}

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
        Ok(self.checkpoint_tree.insert(&epoch, &entry)?)
    }

    fn get_checkpoint(&self, batchidx: u64) -> DbResult<Option<CheckpointEntry>> {
        Ok(self.checkpoint_tree.get(&batchidx)?)
    }

    fn get_last_checkpoint_idx(&self) -> DbResult<Option<u64>> {
        Ok(self.checkpoint_tree.last()?.map(first))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::checkpoint_db_tests;

    use super::*;

    fn setup_db() -> CheckpointDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        CheckpointDBSled::new(sled_db.into(), config).unwrap()
    }

    checkpoint_db_tests!(setup_db());
}
