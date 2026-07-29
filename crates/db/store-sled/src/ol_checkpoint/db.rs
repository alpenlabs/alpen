use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::common::L1PayloadIntentIndex;
use strata_db_types::ol_checkpoint::OLCheckpointDatabase;
use strata_db_types::{DbError, DbResult};
use strata_identifiers::{Epoch, EpochCommitment};
use typed_sled::error;

use super::schemas::*;
use crate::define_sled_database;

define_sled_database!(
    pub struct OLCheckpointDBSled {
        payload_tree: OLCheckpointPayloadSchema,
        signing_tree: OLCheckpointSigningSchema,
        l1_ref_tree: OLCheckpointL1RefSchema,
        l1_observed_payload_tree: OLCheckpointL1ObservedPayloadSchema,
        unsigned_tree: UnsignedCheckpointIndexSchema,
        epoch_summary_tree: OLEpochSummarySchema,
        l1_ref_epoch_index_tree: OLCheckpointEpochIndexSchema,
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
            (
                &self.payload_tree,
                &self.signing_tree,
                &self.l1_ref_tree,
                &self.unsigned_tree,
            ),
            |(pt, st, lot, ut)| {
                if !pt.contains_key(&epoch)? {
                    return Ok(false);
                }
                let had_signing = st.contains_key(&epoch)?;
                pt.remove(&epoch)?;
                // Payload deletion intentionally cascades to signing for the same commitment.
                st.remove(&epoch)?;
                // Payload deletion intentionally cascades to L1 ref for the same
                // commitment.
                lot.remove(&epoch)?;
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
            (
                &self.payload_tree,
                &self.signing_tree,
                &self.l1_ref_tree,
                &self.unsigned_tree,
            ),
            |(pt, st, lot, ut)| {
                let mut deleted_epochs = Vec::new();
                for epoch_comm in &keys {
                    if pt.contains_key(epoch_comm)? {
                        let had_signing = st.contains_key(epoch_comm)?;
                        pt.remove(epoch_comm)?;
                        // Payload deletion intentionally cascades to signing.
                        st.remove(epoch_comm)?;
                        // Payload deletion intentionally cascades to L1 ref.
                        lot.remove(epoch_comm)?;
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

    fn del_local_checkpoint_payload_entries_from_epoch(
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

    fn put_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()> {
        self.config.with_retry(
            (&self.l1_ref_tree, &self.l1_ref_epoch_index_tree),
            |(lot, idx)| {
                lot.insert(&epoch, &l1_ref)?;
                let mut candidates = idx.get(&epoch.epoch())?.unwrap_or_default();
                if !candidates.contains(&epoch) {
                    candidates.push(epoch);
                    idx.insert(&epoch.epoch(), &candidates)?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<Option<CheckpointL1Ref>> {
        Ok(self.l1_ref_tree.get(&epoch)?)
    }

    fn get_last_checkpoint_l1_ref_epoch(&self) -> DbResult<Option<EpochCommitment>> {
        // Scan to avoid maintaining an additional epoch index table; not on a
        // hot write path.
        let mut max_commitment: Option<EpochCommitment> = None;
        for item in self.l1_ref_tree.iter() {
            let (commitment, _l1_ref) = item?;
            max_commitment = Some(match max_commitment {
                None => commitment,
                Some(current) if commitment.epoch() > current.epoch() => commitment,
                Some(current) => current,
            });
        }
        Ok(max_commitment)
    }

    fn get_checkpoint_l1_refs_from(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<(EpochCommitment, CheckpointL1Ref)>> {
        let mut refs = Vec::new();
        for item in self.l1_ref_tree.iter() {
            let (commitment, l1_ref) = item?;
            if commitment.epoch() >= start_epoch {
                refs.push((commitment, l1_ref));
            }
        }
        refs.sort_by_key(|(commitment, _)| commitment.epoch());
        Ok(refs)
    }

    fn get_observed_checkpoint_commitments_for_epoch(
        &self,
        epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        if let Some(candidates) = self.l1_ref_epoch_index_tree.get(&epoch)? {
            return Ok(candidates);
        }

        // Read-repair: a legacy entry written before the index existed. Rebuild
        // it from the L1-ref table and persist so later reads hit the index. The
        // scan is non-transactional; the persist unions the scanned candidates
        // with any entry created concurrently so neither set is lost.
        let scanned: Vec<EpochCommitment> = self
            .get_checkpoint_l1_refs_from(epoch)?
            .into_iter()
            .filter(|(commitment, _)| commitment.epoch() == epoch)
            .map(|(commitment, _)| commitment)
            .collect();

        self.config
            .with_retry((&self.l1_ref_epoch_index_tree,), |(idx,)| {
                let mut merged = idx.get(&epoch)?.unwrap_or_default();
                for commitment in &scanned {
                    if !merged.contains(commitment) {
                        merged.push(*commitment);
                    }
                }
                idx.insert(&epoch, &merged)?;
                Ok(merged)
            })
    }

    fn del_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.config.with_retry(
            (&self.l1_ref_tree, &self.l1_ref_epoch_index_tree),
            |(lot, idx)| {
                if !lot.contains_key(&epoch)? {
                    return Ok(false);
                }
                lot.remove(&epoch)?;
                if let Some(mut candidates) = idx.get(&epoch.epoch())? {
                    candidates.retain(|c| c != &epoch);
                    if candidates.is_empty() {
                        idx.remove(&epoch.epoch())?;
                    } else {
                        idx.insert(&epoch.epoch(), &candidates)?;
                    }
                }
                Ok(true)
            },
        )
    }

    fn del_checkpoint_l1_refs_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        let mut keys = Vec::new();
        for item in self.l1_ref_tree.iter() {
            let (epoch_comm, _entry) = item?;
            if epoch_comm.epoch() >= start_epoch {
                keys.push(epoch_comm);
            }
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let deleted_epochs = self.config.with_retry(
            (&self.l1_ref_tree, &self.l1_ref_epoch_index_tree),
            |(lot, idx)| {
                let mut deleted_epochs = Vec::new();
                for epoch_comm in &keys {
                    if lot.contains_key(epoch_comm)? {
                        lot.remove(epoch_comm)?;
                        idx.remove(&epoch_comm.epoch())?;
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

    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()> {
        let expected_commitment = EpochCommitment::from_terminal(
            Epoch::from(payload.new_tip().epoch),
            *payload.new_tip().l2_commitment(),
        );
        if commitment != expected_commitment {
            return Err(DbError::InvalidArgument);
        }

        self.config.with_retry(
            (
                &self.l1_observed_payload_tree,
                &self.l1_ref_tree,
                &self.l1_ref_epoch_index_tree,
            ),
            |(opt, lot, idx)| {
                opt.insert(&commitment, &payload)?;
                lot.insert(&commitment, &l1_ref)?;
                let mut candidates = idx.get(&commitment.epoch())?.unwrap_or_default();
                if !candidates.contains(&commitment) {
                    candidates.push(commitment);
                    idx.insert(&commitment.epoch(), &candidates)?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_checkpoint_l1_observed_payload(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        Ok(self.l1_observed_payload_tree.get(&epoch)?)
    }

    fn del_checkpoint_l1_observed_payload(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.config
            .with_retry((&self.l1_observed_payload_tree,), |(opt,)| {
                if !opt.contains_key(&epoch)? {
                    return Ok(false);
                }
                opt.remove(&epoch)?;
                Ok(true)
            })
    }

    fn del_checkpoint_l1_observed_payloads_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>> {
        let mut keys = Vec::new();
        for item in self.l1_observed_payload_tree.iter() {
            let (epoch_comm, _entry) = item?;
            if epoch_comm.epoch() >= start_epoch {
                keys.push(epoch_comm);
            }
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let deleted_epochs =
            self.config
                .with_retry((&self.l1_observed_payload_tree,), |(opt,)| {
                    let mut deleted_epochs = Vec::new();
                    for epoch_comm in &keys {
                        if opt.contains_key(epoch_comm)? {
                            opt.remove(epoch_comm)?;
                            deleted_epochs.push(*epoch_comm);
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
    use strata_csm_types::CheckpointL1Ref;
    use strata_db_tests::ol_checkpoint_db_tests;
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, OLBlockId, RBuf32};

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLCheckpointDBSled, ol_checkpoint_db_tests);

    fn temp_db() -> OLCheckpointDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = typed_sled::SledDb::new(db).unwrap();
        OLCheckpointDBSled::new(sled_db.into(), crate::SledDbConfig::test()).unwrap()
    }

    fn l1_ref(height: u32) -> CheckpointL1Ref {
        let blkid = L1BlockId::from(Buf32::from([height as u8; 32]));
        CheckpointL1Ref::new(
            L1BlockCommitment::new(height, blkid),
            RBuf32::from([height as u8; 32]),
            RBuf32::from([(height + 1) as u8; 32]),
        )
    }

    fn commitment(epoch: u32, tag: u8) -> EpochCommitment {
        EpochCommitment::new(
            Epoch::from(epoch),
            u64::from(epoch),
            OLBlockId::from(Buf32::from([tag; 32])),
        )
    }

    /// Legacy rows written directly to the L1-ref tree (before the index
    /// existed) are read-repaired on first lookup and then served from the
    /// index. Seeds via a direct tree insert because the public writers now
    /// maintain the index, so only a raw insert reproduces a legacy row.
    #[test]
    fn observed_commitments_read_repair_from_legacy_rows() {
        let db = temp_db();
        let a = commitment(9, 1);
        let b = commitment(9, 2);
        db.l1_ref_tree.insert(&a, &l1_ref(900)).unwrap();
        db.l1_ref_tree.insert(&b, &l1_ref(901)).unwrap();
        assert!(
            db.l1_ref_epoch_index_tree
                .get(&Epoch::from(9u32))
                .unwrap()
                .is_none(),
            "index must be absent before repair"
        );

        // First lookup repairs from the scan and persists the candidate set.
        let mut repaired = db
            .get_observed_checkpoint_commitments_for_epoch(Epoch::from(9u32))
            .unwrap();
        repaired.sort_by_key(|c| *c.last_blkid());
        let mut expected = vec![a, b];
        expected.sort_by_key(|c| *c.last_blkid());
        assert_eq!(repaired, expected);

        let mut persisted = db
            .l1_ref_epoch_index_tree
            .get(&Epoch::from(9u32))
            .unwrap()
            .expect("index persisted after repair");
        persisted.sort_by_key(|c| *c.last_blkid());
        assert_eq!(persisted, expected);
    }

    /// Confirms the per-epoch lookup serves from the index without rescanning the
    /// L1-ref table on every call: once the index is populated, the result no
    /// longer depends on the underlying l1_refs, so a cold-start catch-up that
    /// resolves many predecessors does not repeat the full scan per epoch.
    #[test]
    fn observed_commitments_served_from_index_without_rescan() {
        let db = temp_db();
        let c = commitment(7, 1);
        db.put_checkpoint_l1_ref(c, l1_ref(700)).unwrap();

        // The public writer populated the index, so this lookup is index-served.
        assert_eq!(
            db.get_observed_checkpoint_commitments_for_epoch(Epoch::from(7u32))
                .unwrap(),
            vec![c]
        );

        // Drop the l1_ref tree entry but leave the index intact. A rescan would
        // now return empty; serving from the index still returns the candidate.
        db.l1_ref_tree.remove(&c).unwrap();
        assert_eq!(
            db.get_observed_checkpoint_commitments_for_epoch(Epoch::from(7u32))
                .unwrap(),
            vec![c],
            "second lookup must hit the index, not rescan the l1_ref table"
        );
    }
}
