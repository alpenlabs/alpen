use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::{
    DbError, DbResult, traits::OLCheckpointDatabase, types::L1PayloadIntentIndex,
};
use strata_identifiers::{Epoch, EpochCommitment};
use typed_sled::error;

use super::schemas::*;
use crate::define_sled_database;

define_sled_database!(
    pub struct OLCheckpointDBSled {
        payload_tree: OLCheckpointPayloadSchema,
        signing_tree: OLCheckpointSigningSchema,
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

    fn get_epoch_commitments_at(&self, epoch: Epoch) -> DbResult<Vec<EpochCommitment>> {
        let summaries = self
            .epoch_summary_tree
            .get(&u64::from(epoch))?
            .unwrap_or_else(Vec::new);
        Ok(summaries
            .into_iter()
            .map(|s| s.get_epoch_commitment())
            .collect::<Vec<_>>())
    }

    fn get_last_summarized_epoch(&self) -> DbResult<Option<Epoch>> {
        Ok(self.epoch_summary_tree.last()?.map(|(e, _)| e as Epoch))
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

    fn del_epoch_summaries_from_epoch(&self, start_epoch: Epoch) -> DbResult<Vec<EpochCommitment>> {
        let last_epoch = self.get_last_summarized_epoch()?;
        let Some(last_epoch) = last_epoch else {
            return Ok(Vec::new());
        };

        if start_epoch > last_epoch {
            return Ok(Vec::new());
        }

        let deleted_commitments =
            self.config
                .with_retry((&self.epoch_summary_tree,), |(est,)| {
                    let mut deleted_commitments = Vec::new();
                    for epoch in start_epoch..=last_epoch {
                        let key = u64::from(epoch);
                        if let Some(summaries) = est.get(&key)? {
                            est.remove(&key)?;
                            deleted_commitments.extend(
                                summaries
                                    .into_iter()
                                    .map(|summary| summary.get_epoch_commitment()),
                            );
                        }
                    }
                    Ok(deleted_commitments)
                })?;
        Ok(deleted_commitments)
    }

    fn put_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
        payload: CheckpointPayload,
    ) -> DbResult<()> {
        let expected_commitment = EpochCommitment::from_terminal(
            Epoch::from(payload.new_tip().epoch),
            *payload.new_tip().l2_commitment(),
        );
        if epoch != expected_commitment {
            return Err(DbError::InvalidArgument);
        }

        let epoch_num = epoch.epoch();
        self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                pt.insert(&epoch, &payload)?;
                if !st.contains_key(&epoch)? {
                    ut.insert(&epoch_num, &())?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        Ok(self.payload_tree.get(&epoch)?)
    }

    fn get_last_checkpoint_payload_epoch(&self) -> DbResult<Option<EpochCommitment>> {
        // We intentionally keep this as a scan to avoid maintaining an additional
        // payload-epoch index table. This query is not on a hot write path.
        let mut max_commitment: Option<EpochCommitment> = None;
        for item in self.payload_tree.iter() {
            let (commitment, _payload) = item?;
            max_commitment = Some(match max_commitment {
                None => commitment,
                Some(current) if commitment.epoch() > current.epoch() => commitment,
                Some(current) => current,
            });
        }
        Ok(max_commitment)
    }

    fn del_checkpoint_payload_entry(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let epoch_num = epoch.epoch();
        self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                if !pt.contains_key(&epoch)? {
                    return Ok(false);
                }
                let had_signing = st.contains_key(&epoch)?;
                pt.remove(&epoch)?;
                // Payload deletion intentionally cascades to signing for the same commitment.
                st.remove(&epoch)?;
                if !had_signing {
                    ut.remove(&epoch_num)?;
                }
                Ok(true)
            },
        )
    }

    fn del_checkpoint_payload_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        let mut keys = Vec::new();
        for item in self.payload_tree.iter() {
            let (epoch_comm, _entry) = item?;
            if epoch_comm.epoch() >= start_epoch {
                keys.push(epoch_comm);
            }
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let deleted_epochs = self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                let mut deleted_epochs = Vec::new();
                for epoch_comm in &keys {
                    if pt.contains_key(epoch_comm)? {
                        let had_signing = st.contains_key(epoch_comm)?;
                        pt.remove(epoch_comm)?;
                        // Payload deletion intentionally cascades to signing.
                        st.remove(epoch_comm)?;
                        if !had_signing {
                            ut.remove(&epoch_comm.epoch())?;
                        }
                        deleted_epochs.push(*epoch_comm);
                    }
                }
                Ok(deleted_epochs)
            },
        )?;
        Ok(deleted_epochs)
    }

    fn put_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
        payload_intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()> {
        self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                if !pt.contains_key(&epoch)? {
                    return Err(error::ConflictableTransactionError::Abort(
                        error::Error::abort(DbError::InvalidArgument),
                    ));
                }
                let was_signed = st.contains_key(&epoch)?;
                st.insert(&epoch, &payload_intent_idx)?;
                if !was_signed {
                    ut.remove(&epoch.epoch())?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<L1PayloadIntentIndex>> {
        Ok(self.signing_tree.get(&epoch)?)
    }

    fn del_checkpoint_signing_entry(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                let existing = st.get(&epoch)?;
                if existing.is_none() {
                    return Ok(false);
                }
                st.remove(&epoch)?;
                if pt.contains_key(&epoch)? {
                    ut.insert(&epoch.epoch(), &())?;
                }
                Ok(true)
            },
        )
    }

    fn del_checkpoint_signing_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        let mut keys = Vec::new();
        for item in self.signing_tree.iter() {
            let (epoch_comm, _entry) = item?;
            if epoch_comm.epoch() >= start_epoch {
                keys.push(epoch_comm);
            }
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let deleted_epochs = self.config.with_retry(
            (&self.payload_tree, &self.signing_tree, &self.unsigned_tree),
            |(pt, st, ut)| {
                let mut deleted_epochs = Vec::new();
                for epoch_comm in &keys {
                    if st.contains_key(epoch_comm)? {
                        st.remove(epoch_comm)?;
                        if pt.contains_key(epoch_comm)? {
                            ut.insert(&epoch_comm.epoch(), &())?;
                        }
                        deleted_epochs.push(*epoch_comm);
                    }
                }
                Ok(deleted_epochs)
            },
        )?;
        Ok(deleted_epochs)
    }

    fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        let mut iter = self.unsigned_tree.iter();
        Ok(iter.next().transpose()?.map(|(epoch, _)| epoch))
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
