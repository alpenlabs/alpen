use sled::transaction::ConflictableTransactionError;
use strata_db_types::ol_block::{BlockAvailability, BlockStatus, OLBlockDatabase};
use strata_db_types::{DbError, DbResult};
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types::{OLBlock, OLBlockHeader};
use typed_sled::error::Error as TSledError;

use super::schemas::{
    OLBlockHeightSchema, OLBlockHighWatermarkSchema, OLBlockSchema, OLBlockStatusSchema,
    OLCanonicalBlockSchema, OLHistoryBaseSchema, OLTerminalHeaderSchema,
};
use crate::define_sled_database;
use crate::utils::{first, to_db_error};

const OL_BLOCK_HIGH_WATERMARK_KEY: u8 = 0;
const OL_HISTORY_BASE_KEY: u8 = 0;

define_sled_database!(
    pub struct OLBlockDBSled {
        blk_tree: OLBlockSchema,
        terminal_header_tree: OLTerminalHeaderSchema,
        blk_status_tree: OLBlockStatusSchema,
        blk_height_tree: OLBlockHeightSchema,
        blk_high_watermark_tree: OLBlockHighWatermarkSchema,
        blk_canonical_tree: OLCanonicalBlockSchema,
        history_base_tree: OLHistoryBaseSchema,
    }
);

impl OLBlockDatabase for OLBlockDBSled {
    fn put_block_data(&self, block: OLBlock) -> DbResult<()> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                    let is_new = !blocks_at_slot.contains(&block_id);

                    if is_new {
                        blocks_at_slot.push(block_id);
                        bht.insert(&slot, &blocks_at_slot)?;

                        // Only set status to Unchecked for new blocks
                        // This preserves Valid/Invalid status if block is re-inserted
                        bst.insert(&block_id, &BlockStatus::Unchecked)?;
                    }

                    bt.insert(&block_id, &block)?;
                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self
            .blk_high_watermark_tree
            .get(&OL_BLOCK_HIGH_WATERMARK_KEY)?)
    }

    fn put_block_data_with_high_watermark(&self, block: OLBlock) -> DbResult<OLBlockCommitment> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(slot, block_id);

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.blk_high_watermark_tree,
            ),
            |(bt, bst, bht, hwt)| {
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
            .map_err(to_db_error)
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

    fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        Ok(self.blk_tree.get(&id)?)
    }

    fn put_terminal_header(&self, id: OLBlockId, header: OLBlockHeader) -> DbResult<()> {
        let computed = header.compute_blkid();
        if computed != id {
            return Err(DbError::OLTerminalHeaderIdMismatch { key: id, computed });
        }

        self.terminal_header_tree.insert(&id, &header)?;
        Ok(())
    }

    fn get_terminal_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeader>> {
        Ok(self.terminal_header_tree.get(&id)?)
    }

    fn get_ol_header(&self, id: OLBlockId) -> DbResult<Option<OLBlockHeader>> {
        if let Some(block) = self.get_block_data(id)? {
            return Ok(Some(block.header().clone()));
        }

        self.get_terminal_header(id)
    }

    fn get_history_base(&self) -> DbResult<Option<EpochCommitment>> {
        Ok(self.history_base_tree.get(&OL_HISTORY_BASE_KEY)?)
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
        for item in self.blk_canonical_tree.range(anchor.last_slot()..)? {
            let (slot, _) = item?;
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
        if self.blk_canonical_tree.get(&slot)? == Some(id) {
            for item in self.blk_canonical_tree.range(slot..)? {
                let (canonical_slot, _) = item?;
                canonical_slots_to_drop.push(canonical_slot);
            }
        }

        self.config
            .with_retry(
                (
                    &self.blk_tree,
                    &self.blk_status_tree,
                    &self.blk_height_tree,
                    &self.blk_canonical_tree,
                ),
                |(bt, bst, bht, ct)| {
                    let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                    blocks_at_slot.retain(|&bid| bid != id);

                    bt.remove(&id)?;
                    bst.remove(&id)?;
                    bht.insert(&slot, &blocks_at_slot)?;
                    if ct.get(&slot)? == Some(id) {
                        for canonical_slot in &canonical_slots_to_drop {
                            ct.remove(canonical_slot)?;
                        }
                    }
                    Ok(true)
                },
            )
            .map_err(to_db_error)
    }

    fn set_block_status(&self, id: OLBlockId, status: BlockStatus) -> DbResult<bool> {
        // Check if block exists before setting status
        if self.get_block_data(id)?.is_none() {
            return Err(DbError::NonExistentEntry);
        }
        self.blk_status_tree.insert(&id, &status)?;
        Ok(true)
    }

    fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        Ok(self.blk_height_tree.get(&slot)?.unwrap_or(Vec::new()))
    }

    fn get_block_status(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        Ok(self.blk_status_tree.get(&id)?)
    }

    fn get_tip_slot(&self) -> DbResult<Slot> {
        self.blk_canonical_tree
            .last()?
            .map(first)
            .ok_or(DbError::NotBootstrapped)
    }

    fn get_canonical_block(&self, slot: Slot) -> DbResult<Option<OLBlockId>> {
        Ok(self.blk_canonical_tree.get(&slot)?)
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
        for item in self.blk_canonical_tree.range(start_slot..)? {
            let (slot, _) = item?;
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
            .map_err(to_db_error)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::ol_block_db_tests;
    use strata_ol_chain_types::test_utils as ol_test_utils;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLBlockDBSled, ol_block_db_tests);

    proptest::proptest! {
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
