use std::ops::Bound::{Excluded, Included, Unbounded};

use sled::transaction::ConflictableTransactionError;
use strata_db_types::ol_block::{BlockAvailability, BlockStatus, OLBlockDatabase};
use strata_db_types::{DbError, DbResult};
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use typed_sled::error::Error as TSledError;

use super::schemas::{
    OLBlockHeightSchema, OLBlockHighWatermarkSchema, OLBlockRejectionCompleteSchema, OLBlockSchema,
    OLBlockStatusSchema, OLCanonicalBlockSchema, OLHistoryBaseSchema, OLTerminalHeaderSchema,
};
use crate::define_sled_database;
use crate::utils::{conv_sled_err, first};

const OL_BLOCK_HIGH_WATERMARK_KEY: u8 = 0;
const OL_HISTORY_BASE_KEY: u8 = 0;

define_sled_database!(
    pub struct OLBlockDBSled {
        blk_tree: OLBlockSchema,
        terminal_header_tree: OLTerminalHeaderSchema,
        blk_status_tree: OLBlockStatusSchema,
        rejection_complete_tree: OLBlockRejectionCompleteSchema,
        blk_height_tree: OLBlockHeightSchema,
        blk_high_watermark_tree: OLBlockHighWatermarkSchema,
        blk_canonical_tree: OLCanonicalBlockSchema,
        history_base_tree: OLHistoryBaseSchema,
    }
);

impl OLBlockDatabase for OLBlockDBSled {
    fn put_block_data(&self, block: OLBlockV1) -> DbResult<()> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.rejection_complete_tree,
            ),
            |(bt, bst, bht, rct)| {
                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                let is_new = !blocks_at_slot.contains(&block_id);

                if is_new {
                    blocks_at_slot.push(block_id);
                    bht.insert(&slot, &blocks_at_slot)?;

                    // Only set status to Unchecked for new blocks
                    // This preserves Valid/Invalid status if block is re-inserted
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                    rct.remove(&block_id)?;
                }

                bt.insert(&block_id, &block)?;
                Ok(())
            },
        )?;
        Ok(())
    }

    fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.blk_high_watermark_tree
            .get(&OL_BLOCK_HIGH_WATERMARK_KEY)
            .map_err(conv_sled_err)
    }

    fn put_block_data_with_high_watermark(&self, block: OLBlockV1) -> DbResult<OLBlockCommitment> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(slot, block_id);

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.blk_high_watermark_tree,
                &self.rejection_complete_tree,
            ),
            |(bt, bst, bht, hwt, rct)| {
                if let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)?
                    && commitment.slot() <= current.slot()
                {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::BlockHighWatermarkConflict {
                            attempted: commitment,
                            current,
                        },
                    )));
                }

                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                let is_new = !blocks_at_slot.contains(&block_id);

                if is_new {
                    blocks_at_slot.push(block_id);
                    bht.insert(&slot, &blocks_at_slot)?;

                    // Only set status to Unchecked for new blocks.
                    // This preserves Valid/Invalid status if block is re-inserted.
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                }

                bt.insert(&block_id, &block)?;
                hwt.insert(&OL_BLOCK_HIGH_WATERMARK_KEY, &commitment)?;
                // Reoffering an invalid block requires clearing its new high-watermark too.
                rct.remove(&block_id)?;

                Ok(commitment)
            },
        )
    }

    fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
        self.config
            .with_retry((&self.blk_high_watermark_tree,), |(hwt,)| {
                let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)? else {
                    return Ok(false);
                };

                if current != expected {
                    return Ok(false);
                }

                hwt.remove(&OL_BLOCK_HIGH_WATERMARK_KEY)?;
                Ok(true)
            })
    }

    fn rollback_block_high_watermark(&self, target: OLBlockCommitment) -> DbResult<bool> {
        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_high_watermark_tree,
                &self.rejection_complete_tree,
            ),
            |(bt, hwt, rct)| {
                let target_block_id = *target.blkid();
                let Some(target_block) = bt.get(&target_block_id)? else {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::NonExistentEntry,
                    )));
                };

                if target_block.header().slot() != target.slot() {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::InvalidArgument,
                    )));
                }

                let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)? else {
                    return Ok(false);
                };

                if current.slot() <= target.slot() {
                    return Ok(false);
                }

                hwt.insert(&OL_BLOCK_HIGH_WATERMARK_KEY, &target)?;
                rct.remove(&target_block_id)?;
                Ok(true)
            },
        )
    }

    fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlockV1>> {
        self.blk_tree.get(&id).map_err(conv_sled_err)
    }

    fn put_terminal_header(&self, id: OLBlockId, header: OLBlockHeaderV1) -> DbResult<()> {
        let computed = header.compute_blkid();
        if computed != id {
            return Err(DbError::OLTerminalHeaderIdMismatch { key: id, computed });
        }

        self.terminal_header_tree
            .insert(&id, &header)
            .map_err(conv_sled_err)?;
        Ok(())
    }

    fn get_terminal_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeaderV1>> {
        self.terminal_header_tree.get(&id).map_err(conv_sled_err)
    }

    fn get_ol_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeaderV1>> {
        if let Some(block) = self.get_block_data(id)? {
            return Ok(Some(block.header().clone()));
        }

        self.get_terminal_header(id)
    }

    fn get_history_base(&self) -> DbResult<Option<EpochCommitment>> {
        self.history_base_tree
            .get(&OL_HISTORY_BASE_KEY)
            .map_err(conv_sled_err)
    }

    fn get_block_at(&self, commitment: OLBlockCommitment) -> DbResult<BlockAvailability> {
        if let Some(block) = self.get_block_data(*commitment.blkid())? {
            return Ok(BlockAvailability::Available(Box::new(block)));
        }

        match self.get_history_base()? {
            Some(base) if commitment.slot() <= base.last_slot() => Ok(BlockAvailability::Pruned),
            _ => Ok(BlockAvailability::Missing),
        }
    }

    fn promote_to_history_anchor(&self, anchor: EpochCommitment) -> DbResult<()> {
        if let Some(current) = self.get_history_base()? {
            return if current == anchor {
                Ok(())
            } else {
                Err(DbError::OLHistoryBaseConflict {
                    attempted: anchor,
                    current,
                })
            };
        }

        // Collect the suffix slots before the transaction: sled's transactional tree has no range
        // scan.
        let mut slots_to_drop = Vec::new();
        for item in self
            .blk_canonical_tree
            .range(anchor.last_slot()..)
            .map_err(conv_sled_err)?
        {
            let (slot, _) = item.map_err(conv_sled_err)?;
            slots_to_drop.push(slot);
        }

        self.config.with_retry(
            (&self.blk_canonical_tree, &self.history_base_tree),
            |(ct, hbt)| {
                if let Some(current) = hbt.get(&OL_HISTORY_BASE_KEY)? {
                    return if current == anchor {
                        Ok(())
                    } else {
                        Err(ConflictableTransactionError::Abort(TSledError::abort(
                            DbError::OLHistoryBaseConflict {
                                attempted: anchor,
                                current,
                            },
                        )))
                    };
                }

                for slot in &slots_to_drop {
                    ct.remove(slot)?;
                }
                ct.insert(&anchor.last_slot(), anchor.last_blkid())?;
                hbt.insert(&OL_HISTORY_BASE_KEY, &anchor)?;
                Ok(())
            },
        )
    }

    fn del_block_data(&self, id: OLBlockId) -> DbResult<bool> {
        // Need to find which slot this block is at
        let block = match self.get_block_data(id)? {
            Some(b) => b,
            None => return Ok(false),
        };
        let slot = block.header().slot();
        let mut canonical_slots_to_drop = Vec::new();
        if self.blk_canonical_tree.get(&slot).map_err(conv_sled_err)? == Some(id) {
            for item in self
                .blk_canonical_tree
                .range(slot..)
                .map_err(conv_sled_err)?
            {
                let (canonical_slot, _) = item.map_err(conv_sled_err)?;
                canonical_slots_to_drop.push(canonical_slot);
            }
        }

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.blk_canonical_tree,
                &self.rejection_complete_tree,
            ),
            |(bt, bst, bht, ct, rct)| {
                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                blocks_at_slot.retain(|&bid| bid != id);

                bt.remove(&id)?;
                bst.remove(&id)?;
                rct.remove(&id)?;
                if blocks_at_slot.is_empty() {
                    bht.remove(&slot)?;
                } else {
                    bht.insert(&slot, &blocks_at_slot)?;
                }
                if ct.get(&slot)? == Some(id) {
                    for canonical_slot in &canonical_slots_to_drop {
                        ct.remove(canonical_slot)?;
                    }
                }
                Ok(true)
            },
        )
    }

    fn set_block_status(&self, id: OLBlockId, status: BlockStatus) -> DbResult<bool> {
        // Check if block exists before setting status
        if self.get_block_data(id)?.is_none() {
            return Err(DbError::NonExistentEntry);
        }
        self.config.with_retry(
            (&self.blk_status_tree, &self.rejection_complete_tree),
            |(bst, rct)| {
                bst.insert(&id, &status)?;
                if status != BlockStatus::Invalid {
                    rct.remove(&id)?;
                }
                Ok(())
            },
        )?;
        Ok(true)
    }

    fn is_block_rejection_complete(&self, id: OLBlockId) -> DbResult<bool> {
        self.rejection_complete_tree
            .get(&id)
            .map(|complete| complete.unwrap_or(false))
            .map_err(conv_sled_err)
    }

    fn mark_block_rejection_complete(&self, block: OLBlockCommitment) -> DbResult<()> {
        self.config.with_retry(
            (
                &self.blk_status_tree,
                &self.blk_high_watermark_tree,
                &self.rejection_complete_tree,
            ),
            |(bst, hwt, rct)| {
                if bst.get(block.blkid())? != Some(BlockStatus::Invalid)
                    || hwt
                        .get(&OL_BLOCK_HIGH_WATERMARK_KEY)?
                        .is_some_and(|current| current.blkid() == block.blkid())
                {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::InvalidArgument,
                    )));
                }
                rct.insert(block.blkid(), &true)?;
                Ok(())
            },
        )
    }

    fn scan_block_rejection_statuses(
        &self,
        after: Option<OLBlockId>,
        limit: usize,
    ) -> DbResult<Vec<(OLBlockId, bool)>> {
        let start = after.map_or(Unbounded, Excluded);
        self.blk_status_tree
            .range((start, Unbounded))
            .map_err(conv_sled_err)?
            .take(limit)
            .map(|row| {
                let (id, status) = row.map_err(conv_sled_err)?;
                // An unreadable completion marker must not stall discovery of later rows.
                // Cleanup retries the read with backoff before performing any writes.
                let pending = status == BlockStatus::Invalid
                    && !self.is_block_rejection_complete(id).unwrap_or(false);
                Ok((id, pending))
            })
            .collect()
    }

    fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        Ok(self
            .blk_height_tree
            .get(&slot)
            .map_err(conv_sled_err)?
            .unwrap_or(Vec::new()))
    }

    fn scan_block_statuses(
        &self,
        finalized_slot: Slot,
        after: Option<OLBlockCommitment>,
        limit: usize,
    ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let after = after.filter(|block| block.slot() > finalized_slot);
        let start = after.map_or(Excluded(finalized_slot), |block| Included(block.slot()));
        let mut rows = Vec::new();
        for row in self
            .blk_height_tree
            .range((start, Unbounded))
            .map_err(conv_sled_err)?
        {
            let (slot, mut ids) = row.map_err(conv_sled_err)?;
            ids.sort_unstable();
            for id in ids {
                let block = OLBlockCommitment::new(slot, id);
                if after.is_some_and(|after| block <= after) {
                    continue;
                }
                if let Some(status) = self.blk_status_tree.get(&id).map_err(conv_sled_err)? {
                    rows.push((block, status));
                    if rows.len() == limit {
                        return Ok(rows);
                    }
                }
            }
        }
        Ok(rows)
    }

    fn get_highest_block_slot(&self) -> DbResult<Option<Slot>> {
        // Skip empty height rows: older datadirs may retain rows whose last
        // block was deleted before empty rows were removed on delete.
        for item in self.blk_height_tree.iter().rev() {
            let (slot, ids) = item.map_err(conv_sled_err)?;
            if !ids.is_empty() {
                return Ok(Some(slot));
            }
        }
        Ok(None)
    }

    fn get_block_status(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.blk_status_tree.get(&id).map_err(conv_sled_err)
    }

    fn get_tip_slot(&self) -> DbResult<Slot> {
        self.blk_canonical_tree
            .last()
            .map_err(conv_sled_err)?
            .map(first)
            .ok_or(DbError::NotBootstrapped)
    }

    fn get_canonical_block(&self, slot: Slot) -> DbResult<Option<OLBlockId>> {
        self.blk_canonical_tree.get(&slot).map_err(conv_sled_err)
    }

    fn replace_canonical_suffix_from(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()> {
        let block_count = block_ids.len();
        let mut blocks = Vec::with_capacity(block_count);
        for (offset, block_id) in block_ids.into_iter().enumerate() {
            let offset = u64::try_from(offset).map_err(|_| DbError::OLCanonicalSuffixOverflow {
                start_slot,
                block_count,
            })?;
            let slot =
                start_slot
                    .checked_add(offset)
                    .ok_or(DbError::OLCanonicalSuffixOverflow {
                        start_slot,
                        block_count,
                    })?;
            blocks.push((slot, block_id));
        }

        // Collect the suffix slots before the transaction: sled's transactional tree has no range
        // scan.
        let mut slots_to_drop = Vec::new();
        for item in self
            .blk_canonical_tree
            .range(start_slot..)
            .map_err(conv_sled_err)?
        {
            let (slot, _) = item.map_err(conv_sled_err)?;
            slots_to_drop.push(slot);
        }

        // Now actually remove and insert new canonical blocks inside a transaction.
        self.config
            .with_retry((&self.blk_canonical_tree,), |(ct,)| {
                for slot in &slots_to_drop {
                    ct.remove(slot)?;
                }
                for (slot, id) in &blocks {
                    ct.insert(slot, id)?;
                }
                Ok(())
            })
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sled::Config;
    use strata_db_tests::ol_block_db_tests;
    use strata_ol_chain_types_v1::test_utils as ol_test_utils;
    use typed_sled::SledDb;
    use typed_sled::codec::KeyCodec;

    use super::*;
    use crate::{SledDbConfig, sled_db_test_setup};

    sled_db_test_setup!(OLBlockDBSled, ol_block_db_tests);

    proptest::proptest! {
        #[test]
        fn rejection_scan_advances_past_unreadable_completion_markers(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let raw_db = Config::new().temporary(true).open().unwrap();
            let raw_markers = raw_db.open_tree("OLBlockRejectionCompleteSchema").unwrap();
            let db = OLBlockDBSled::new(
                Arc::new(SledDb::new(raw_db).unwrap()), SledDbConfig::test(),
            ).unwrap();
            let mut ids = Vec::new();
            for slot in 0..2 {
                block.signed_header.header.slot = slot;
                db.put_block_data(block.clone()).unwrap();
                let id = block.header().compute_blkid();
                db.set_block_status(id, BlockStatus::Invalid).unwrap();
                ids.push(id);
            }
            ids.sort_unstable();
            raw_markers.insert(
                <OLBlockId as KeyCodec<OLBlockRejectionCompleteSchema>>::encode_key(&ids[0]).unwrap(),
                vec![0xff_u8],
            ).unwrap();
            assert!(db.is_block_rejection_complete(ids[0]).is_err());
            assert_eq!(db.scan_block_rejection_statuses(None, 2).unwrap(),
                vec![(ids[0], true), (ids[1], true)]);
            let first = db.scan_block_rejection_statuses(None, 1).unwrap();
            assert_eq!(first, vec![(ids[0], true)]);
            assert_eq!(db.scan_block_rejection_statuses(Some(first[0].0), 1).unwrap(),
                vec![(ids[1], true)]);
        }

        #[test]
        fn rejection_completion_survives_new_handles_and_resets_with_block_state(
            block in ol_test_utils::ol_block_strategy(),
        ) {
            let raw_db = Config::new().temporary(true).open().unwrap();
            let shared_db = Arc::new(SledDb::new(raw_db.clone()).unwrap());
            let db = OLBlockDBSled::new(shared_db.clone(), SledDbConfig::test()).unwrap();
            let commitment = block.header().compute_block_commitment();
            let id = *commitment.blkid();
            db.put_block_data(block.clone()).unwrap();
            assert!(matches!(db.mark_block_rejection_complete(commitment), Err(DbError::InvalidArgument)));
            db.set_block_status(id, BlockStatus::Invalid).unwrap();
            // Existing invalid records have no marker and need one successful cleanup pass.
            assert!(!db.is_block_rejection_complete(id).unwrap());
            db.mark_block_rejection_complete(commitment).unwrap();
            raw_db.flush().unwrap();
            drop(db);

            let db = OLBlockDBSled::new(shared_db, SledDbConfig::test()).unwrap();
            assert!(db.is_block_rejection_complete(id).unwrap());
            assert_eq!(db.get_block_status(id).unwrap(), Some(BlockStatus::Invalid));
            db.set_block_status(id, BlockStatus::Invalid).unwrap();
            db.put_block_data(block.clone()).unwrap();
            assert!(db.is_block_rejection_complete(id).unwrap());

            for status in [BlockStatus::Valid, BlockStatus::Unchecked] {
                db.set_block_status(id, status).unwrap();
                assert!(!db.is_block_rejection_complete(id).unwrap());
                db.set_block_status(id, BlockStatus::Invalid).unwrap();
                db.mark_block_rejection_complete(commitment).unwrap();
            }

            db.put_block_data_with_high_watermark(block.clone()).unwrap();
            assert!(!db.is_block_rejection_complete(id).unwrap());
            assert!(matches!(db.mark_block_rejection_complete(commitment), Err(DbError::InvalidArgument)));
            assert!(db.clear_block_high_watermark(commitment).unwrap());
            db.mark_block_rejection_complete(commitment).unwrap();
            assert!(db.del_block_data(id).unwrap());
            assert!(!db.is_block_rejection_complete(id).unwrap());
            db.put_block_data(block).unwrap();
            assert_eq!(db.get_block_status(id).unwrap(), Some(BlockStatus::Unchecked));
            assert!(!db.is_block_rejection_complete(id).unwrap());
        }

        #[test]
        fn rollback_watermark_reopens_rejection_cleanup(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            block.signed_header.header.slot = 1;
            let target = block.header().compute_block_commitment();
            db.put_block_data(block.clone()).unwrap();
            db.set_block_status(*target.blkid(), BlockStatus::Invalid).unwrap();
            db.mark_block_rejection_complete(target).unwrap();

            block.signed_header.header.slot = 2;
            db.put_block_data_with_high_watermark(block).unwrap();
            assert!(db.rollback_block_high_watermark(target).unwrap());
            assert!(!db.is_block_rejection_complete(*target.blkid()).unwrap());
            assert!(matches!(db.mark_block_rejection_complete(target), Err(DbError::InvalidArgument)));
        }

        #[test]
        fn rejection_scan_advances_through_nonmatching_pages_without_block_bodies(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            let mut commitments = Vec::new();
            for slot in 0..5 {
                block.signed_header.header.slot = slot;
                db.put_block_data(block.clone()).unwrap();
                let commitment = block.header().compute_block_commitment();
                db.set_block_status(*commitment.blkid(), BlockStatus::Valid).unwrap();
                commitments.push(commitment);
            }
            commitments.sort_by_key(|block| *block.blkid());
            let completed = commitments[1];
            let pending = commitments[4];
            db.set_block_status(*completed.blkid(), BlockStatus::Invalid).unwrap();
            db.mark_block_rejection_complete(completed).unwrap();
            // Legacy invalid rows with no completion marker remain discoverable.
            db.set_block_status(*pending.blkid(), BlockStatus::Invalid).unwrap();
            assert!(db.scan_block_rejection_statuses(None, 0).unwrap().is_empty());
            let first = db.scan_block_rejection_statuses(None, 2).unwrap();
            assert_eq!(first, vec![(*commitments[0].blkid(), false), (*completed.blkid(), false)]);
            db.del_block_data(*completed.blkid()).unwrap();
            // Discovery must use only metadata even when bodies cannot be loaded.
            for commitment in &commitments {
                db.blk_tree.remove(commitment.blkid()).unwrap();
            }
            let second = db.scan_block_rejection_statuses(Some(first[1].0), 2).unwrap();
            assert_eq!(second, vec![(*commitments[2].blkid(), false), (*commitments[3].blkid(), false)]);
            let third = db.scan_block_rejection_statuses(Some(second[1].0), 2).unwrap();
            assert_eq!(third, vec![(*pending.blkid(), true)]);
            assert!(db.scan_block_rejection_statuses(Some(third[0].0), 2).unwrap().is_empty());
        }

        #[test]
        fn status_scan_advances_across_checked_pages(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            let mut expected = Vec::new();
            for (epoch, slot) in [1, 2, 2, 4, 5].into_iter().enumerate() {
                block.signed_header.header.epoch = epoch as u32;
                block.signed_header.header.slot = slot;
                let id = block.header().compute_blkid();
                db.put_block_data(block.clone()).unwrap();
                db.set_block_status(id, BlockStatus::Valid).unwrap();
                expected.push((OLBlockCommitment::new(slot, id), BlockStatus::Valid));
            }
            expected.sort_by_key(|(id, _)| *id);
            // Place the only unchecked row after two fully checked pages.
            expected[4].1 = BlockStatus::Unchecked;
            db.set_block_status(*expected[4].0.blkid(), BlockStatus::Unchecked).unwrap();
            assert!(db.scan_block_statuses(0, None, 0).unwrap().is_empty());
            let first = db.scan_block_statuses(0, None, 2).unwrap();
            assert_eq!(first, expected[..2]);
            // Deleting the previous cursor's block must not skip subsequent rows.
            db.del_block_data(*first[1].0.blkid()).unwrap();
            let second = db.scan_block_statuses(0, Some(first[1].0), 2).unwrap();
            assert_eq!(second, expected[2..4]);
            let third = db.scan_block_statuses(0, Some(second[1].0), 2).unwrap();
            assert_eq!(third, expected[4..]);
            assert!(db.scan_block_statuses(0, Some(third[0].0), 2).unwrap().is_empty());

            // Index-only discovery must work even when full bodies are unavailable.
            for (block, _) in &expected {
                db.blk_tree.remove(block.blkid()).unwrap();
            }
            assert_eq!(db.scan_block_statuses(2, None, 10).unwrap(), expected[3..]);
            // Advancing finality clamps a cursor from the previous scan.
            assert_eq!(db.scan_block_statuses(4, Some(first[0].0), 10).unwrap(), expected[4..]);
            assert!(db.scan_block_statuses(Slot::MAX, None, 10).unwrap().is_empty());
        }

        #[test]
        fn get_highest_block_slot_skips_empty_height_rows(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            block.signed_header.header.slot = 3;
            db.put_block_data(block).expect("test: put block at slot 3");
            // Ghost row left behind by deletes that predate empty-row removal.
            db.blk_height_tree
                .insert(&7, &Vec::new())
                .expect("test: seed empty height row");

            assert_eq!(
                db.get_highest_block_slot()
                    .expect("test: highest block slot"),
                Some(3),
                "empty height rows must not count as block records"
            );
        }

        #[test]
        fn get_ol_header_prefers_full_block_before_terminal_record(
            full_block in ol_test_utils::ol_block_strategy(),
            terminal_block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            let block_id = full_block.header().compute_blkid();
            proptest::prop_assume!(terminal_block.header().compute_blkid() != block_id);

            db.put_block_data(full_block.clone()).expect("test: put full block");
            db.terminal_header_tree
                .insert(&block_id, terminal_block.header())
                .expect("test: seed distinct terminal record");

            assert_eq!(
                db.get_ol_header(block_id).expect("test: get preferred full header"),
                Some(full_block.header().clone())
            );

            db.del_block_data(block_id).expect("test: delete full block");
            assert_eq!(
                db.get_ol_header(block_id).expect("test: get fallback terminal header"),
                Some(terminal_block.header().clone())
            );
        }
    }
}
