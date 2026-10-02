use std::num::NonZeroUsize;
use std::ops::Bound::{Excluded, Unbounded};

use sled::transaction::ConflictableTransactionError;
use strata_db_types::ol_block::{BlockAvailability, BlockStatus, OLBlockDatabase, StatusScanStart};
use strata_db_types::{DbError, DbResult};
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use typed_sled::error::Error as TSledError;

use super::schemas::{
    OLBlockHeightSchema, OLBlockHighWatermarkSchema, OLBlockSchema, OLBlockStatusScanReadySchema,
    OLBlockStatusScanSchema, OLBlockStatusSchema, OLCanonicalBlockSchema, OLHistoryBaseSchema,
    OLTerminalHeaderSchema,
};
use crate::define_sled_database;
use crate::utils::{conv_sled_err, first};

const OL_BLOCK_HIGH_WATERMARK_KEY: u8 = 0;
const OL_HISTORY_BASE_KEY: u8 = 0;
const OL_STATUS_SCAN_READY_KEY: u8 = 0;

define_sled_database!(
    pub struct OLBlockDBSled {
        blk_tree: OLBlockSchema,
        terminal_header_tree: OLTerminalHeaderSchema,
        blk_status_tree: OLBlockStatusSchema,
        status_scan_tree: OLBlockStatusScanSchema,
        status_scan_ready_tree: OLBlockStatusScanReadySchema,
        blk_height_tree: OLBlockHeightSchema,
        blk_high_watermark_tree: OLBlockHighWatermarkSchema,
        blk_canonical_tree: OLCanonicalBlockSchema,
        history_base_tree: OLHistoryBaseSchema,
    }
);

impl OLBlockDBSled {
    /// Backfills the seekable status index once before recovery starts.
    ///
    /// Reads legacy slot vectors only during migration. Each transaction copies the current
    /// status, so concurrent updates and deletions cannot be overwritten by a stale snapshot.
    /// New writes populate both indexes. Interrupted or concurrent backfills are safe to repeat.
    pub fn initialize_status_scan_index(&self) -> DbResult<()> {
        if self
            .status_scan_ready_tree
            .get(&OL_STATUS_SCAN_READY_KEY)
            .map_err(conv_sled_err)?
            == Some(true)
        {
            return Ok(());
        }
        for row in self.blk_height_tree.iter() {
            let (slot, ids) = row.map_err(conv_sled_err)?;
            for id in ids {
                self.config.with_retry(
                    (&self.blk_status_tree, &self.status_scan_tree),
                    |(statuses, scan)| {
                        if let Some(status) = statuses.get(&id)? {
                            scan.insert(&OLBlockCommitment::new(slot, id), &status)?;
                        }
                        Ok(())
                    },
                )?;
            }
        }
        self.status_scan_ready_tree
            .insert(&OL_STATUS_SCAN_READY_KEY, &true)
            .map_err(conv_sled_err)
    }
}

impl OLBlockDatabase for OLBlockDBSled {
    fn put_block_data(&self, block: OLBlockV1) -> DbResult<()> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.status_scan_tree,
            ),
            |(bt, bst, bht, sst)| {
                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                let is_new = !blocks_at_slot.contains(&block_id);

                if is_new {
                    blocks_at_slot.push(block_id);
                    bht.insert(&slot, &blocks_at_slot)?;

                    // Only set status to Unchecked for new blocks
                    // This preserves Valid/Invalid status if block is re-inserted
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                }

                let status = bst.get(&block_id)?.unwrap_or(BlockStatus::Unchecked);
                sst.insert(&OLBlockCommitment::new(slot, block_id), &status)?;
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
                &self.status_scan_tree,
            ),
            |(bt, bst, bht, hwt, sst)| {
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

                let status = bst.get(&block_id)?.unwrap_or(BlockStatus::Unchecked);
                sst.insert(&OLBlockCommitment::new(slot, block_id), &status)?;
                bt.insert(&block_id, &block)?;
                hwt.insert(&OL_BLOCK_HIGH_WATERMARK_KEY, &commitment)?;

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
            (&self.blk_tree, &self.blk_high_watermark_tree),
            |(bt, hwt)| {
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
                &self.status_scan_tree,
            ),
            |(bt, bst, bht, ct, sst)| {
                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                blocks_at_slot.retain(|&bid| bid != id);

                bt.remove(&id)?;
                bst.remove(&id)?;
                sst.remove(&OLBlockCommitment::new(slot, id))?;
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
        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.status_scan_tree,
            ),
            |(blocks, statuses, scan)| {
                let Some(block) = blocks.get(&id)? else {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::NonExistentEntry,
                    )));
                };
                statuses.insert(&id, &status)?;
                scan.insert(&OLBlockCommitment::new(block.header().slot(), id), &status)?;
                Ok(true)
            },
        )
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
        start: StatusScanStart,
        limit: NonZeroUsize,
    ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>> {
        // Normal backend startup initializes this eagerly. Direct database handles can
        // also open legacy stores, so complete their one-time backfill before scanning.
        self.initialize_status_scan_index()?;
        let after = match start {
            StatusScanStart::AfterSlot(slot) => {
                OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([u8::MAX; 32])))
            }
            StatusScanStart::AfterBlock(block) => block,
        };
        self.status_scan_tree
            .range((Excluded(after), Unbounded))
            .map_err(conv_sled_err)?
            .take(limit.get())
            .map(|row| row.map_err(conv_sled_err))
            .collect()
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

    use super::*;
    use crate::{SledDbConfig, sled_db_test_setup};

    sled_db_test_setup!(OLBlockDBSled, ol_block_db_tests);

    proptest::proptest! {
        #[test]
        fn status_scan_seeks_within_forked_slots_without_reading_height_vectors(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let raw_db = Config::new().temporary(true).open().unwrap();
            let shared = Arc::new(SledDb::new(raw_db.clone()).unwrap());
            let db = OLBlockDBSled::new(shared.clone(), SledDbConfig::test()).unwrap();
            let mut expected = Vec::new();
            block.signed_header.header.slot = 256;
            for epoch in 0..9 {
                block.signed_header.header.epoch = epoch;
                db.put_block_data(block.clone()).unwrap();
                let commitment = block.header().compute_block_commitment();
                db.set_block_status(*commitment.blkid(), BlockStatus::Valid).unwrap();
                // Reinsertion must retain the existing verdict in both indexes.
                db.put_block_data(block.clone()).unwrap();
                expected.push((commitment, BlockStatus::Valid));
            }
            expected.sort_by_key(|(block, _)| *block);
            db.initialize_status_scan_index().unwrap();
            let limit = NonZeroUsize::new(2).unwrap();
            let first = db.scan_block_statuses(StatusScanStart::AfterSlot(255), limit).unwrap();
            assert_eq!(first, expected[..2]);
            db.del_block_data(*first[1].0.blkid()).unwrap();
            assert!(db.status_scan_tree.get(&first[1].0).unwrap().is_none());
            // Neither later pages nor reopened handles may decode legacy height vectors.
            raw_db.open_tree("OLBlockHeightSchema").unwrap()
                .insert(256_u64.to_be_bytes(), vec![0xff_u8]).unwrap();
            for (commitment, _) in &expected {
                db.blk_tree.remove(commitment.blkid()).unwrap();
            }
            raw_db.flush().unwrap();
            let db = OLBlockDBSled::new(shared, SledDbConfig::test()).unwrap();
            let mut cursor = StatusScanStart::AfterBlock(first[1].0);
            let mut remaining = Vec::new();
            loop {
                let page = db.scan_block_statuses(cursor, limit).unwrap();
                assert!(page.len() <= limit.get());
                let Some((last, _)) = page.last() else { break };
                cursor = StatusScanStart::AfterBlock(*last);
                remaining.extend(page);
            }
            assert_eq!(remaining, expected[2..]);
            assert!(db.scan_block_statuses(StatusScanStart::AfterSlot(256), limit).unwrap().is_empty());
        }

        #[test]
        fn status_scan_backfill_resumes_without_overwriting_newer_verdicts(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let raw_db = Config::new().temporary(true).open().unwrap();
            let shared = Arc::new(SledDb::new(raw_db.clone()).unwrap());
            let db = OLBlockDBSled::new(shared.clone(), SledDbConfig::test()).unwrap();
            let mut blocks = Vec::new();
            for slot in [1, 255, 256] {
                block.signed_header.header.slot = slot;
                db.put_block_data(block.clone()).unwrap();
                let commitment = block.header().compute_block_commitment();
                // Model an old database with no composite index.
                db.status_scan_tree.remove(&commitment).unwrap();
                blocks.push(commitment);
            }
            // A broken later row interrupts migration after earlier rows were indexed.
            let raw_heights = raw_db.open_tree("OLBlockHeightSchema").unwrap();
            raw_heights.insert(257_u64.to_be_bytes(), vec![0xff_u8]).unwrap();
            assert!(db.initialize_status_scan_index().is_err());
            assert_ne!(db.status_scan_ready_tree.get(&OL_STATUS_SCAN_READY_KEY).unwrap(), Some(true));
            assert_eq!(db.status_scan_tree.get(&blocks[0]).unwrap(), Some(BlockStatus::Unchecked));
            db.set_block_status(*blocks[0].blkid(), BlockStatus::Invalid).unwrap();
            db.del_block_data(*blocks[1].blkid()).unwrap();
            // The remaining legacy row can migrate without its body.
            db.status_scan_tree.remove(&blocks[2]).unwrap();
            db.blk_tree.remove(blocks[2].blkid()).unwrap();
            raw_heights.remove(257_u64.to_be_bytes()).unwrap();
            let db = OLBlockDBSled::new(shared, SledDbConfig::test()).unwrap();
            db.initialize_status_scan_index().unwrap();
            assert_eq!(db.scan_block_statuses(StatusScanStart::AfterSlot(0), NonZeroUsize::new(10).unwrap()).unwrap(),
                vec![(blocks[0], BlockStatus::Invalid), (blocks[2], BlockStatus::Unchecked)]);
            assert!(matches!(db.set_block_status(*blocks[1].blkid(), BlockStatus::Valid), Err(DbError::NonExistentEntry)));
            assert!(db.status_scan_tree.get(&blocks[1]).unwrap().is_none());
        }

        #[test]
        fn watermark_writes_preserve_status_scan_entries(
            mut block in ol_test_utils::ol_block_strategy(),
        ) {
            let db = setup_db();
            block.signed_header.header.slot = 1;
            let commitment = block.header().compute_block_commitment();
            db.put_block_data(block.clone()).unwrap();
            db.set_block_status(*commitment.blkid(), BlockStatus::Valid).unwrap();
            db.put_block_data_with_high_watermark(block.clone()).unwrap();
            assert_eq!(db.status_scan_tree.get(&commitment).unwrap(), Some(BlockStatus::Valid));
            block.signed_header.header.slot = 2;
            let next = db.put_block_data_with_high_watermark(block.clone()).unwrap();
            assert_eq!(db.status_scan_tree.get(&next).unwrap(), Some(BlockStatus::Unchecked));
            block.signed_header.header.epoch ^= 1;
            let rejected = block.header().compute_block_commitment();
            assert!(matches!(db.put_block_data_with_high_watermark(block), Err(DbError::BlockHighWatermarkConflict { .. })));
            assert!(db.status_scan_tree.get(&rejected).unwrap().is_none());
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
            let first = db.scan_block_statuses(StatusScanStart::AfterSlot(0), NonZeroUsize::new(2).unwrap()).unwrap();
            assert_eq!(first, expected[..2]);
            // Deleting the previous cursor's block must not skip subsequent rows.
            db.del_block_data(*first[1].0.blkid()).unwrap();
            let second = db.scan_block_statuses(StatusScanStart::AfterBlock(first[1].0), NonZeroUsize::new(2).unwrap()).unwrap();
            assert_eq!(second, expected[2..4]);
            let third = db.scan_block_statuses(StatusScanStart::AfterBlock(second[1].0), NonZeroUsize::new(2).unwrap()).unwrap();
            assert_eq!(third, expected[4..]);
            assert!(db.scan_block_statuses(StatusScanStart::AfterBlock(third[0].0), NonZeroUsize::new(2).unwrap()).unwrap().is_empty());

            // Index-only discovery must work even when full bodies are unavailable.
            for (block, _) in &expected {
                db.blk_tree.remove(block.blkid()).unwrap();
            }
            assert_eq!(db.scan_block_statuses(StatusScanStart::AfterSlot(2), NonZeroUsize::new(10).unwrap()).unwrap(), expected[3..]);
            // The caller can start a new scan after an entire slot.
            assert_eq!(db.scan_block_statuses(StatusScanStart::AfterSlot(4), NonZeroUsize::new(10).unwrap()).unwrap(), expected[4..]);
            assert!(db.scan_block_statuses(StatusScanStart::AfterSlot(Slot::MAX), NonZeroUsize::new(10).unwrap()).unwrap().is_empty());
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
