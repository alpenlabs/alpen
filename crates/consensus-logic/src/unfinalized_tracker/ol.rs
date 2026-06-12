use async_trait::async_trait;
use strata_db_types::{traits::BlockStatus, DbResult};
use strata_identifiers::Slot;
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::OLBlockId;
use strata_storage::OLBlockManager;
use tracing::{debug, error, warn};

use super::UnfinalizedBlockTracker;
use crate::errors::ChainTipError;

#[async_trait]
pub trait UnfinalizedOLBlockSource: Send + Sync {
    async fn get_blocks_at_height(&self, slot: Slot) -> DbResult<Vec<OLBlockId>>;

    async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>>;

    async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>>;
}

#[async_trait]
impl UnfinalizedOLBlockSource for OLBlockManager {
    async fn get_blocks_at_height(&self, slot: Slot) -> DbResult<Vec<OLBlockId>> {
        self.get_blocks_at_height_async(slot).await
    }

    async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.get_block_status_async(blkid).await
    }

    async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>> {
        self.get_block_data_async(blkid).await
    }
}

impl UnfinalizedBlockTracker {
    pub async fn load_unfinalized_ol_blocks_async(
        &mut self,
        source: &(impl UnfinalizedOLBlockSource + ?Sized),
    ) -> anyhow::Result<Vec<OLBlockId>> {
        let mut height = self.finalized_epoch().last_slot() + 1;
        let mut replay_candidates = Vec::new();

        loop {
            let blkids = match source.get_blocks_at_height(height).await {
                Ok(ids) => ids,
                Err(e) => {
                    error!(%height, err = %e, "failed to get new blocks");
                    return Err(e.into());
                }
            };

            if blkids.is_empty() {
                debug!(%height, "found no more blocks, assuming we're past tip");
                break;
            }

            for blkid in blkids {
                // Check the status so we can skip trying to attach blocks we
                // don't care about.
                match source.get_block_status(blkid).await {
                    Ok(Some(BlockStatus::Valid)) => {}
                    Ok(Some(BlockStatus::Unchecked)) => {
                        if source.get_ol_block(blkid).await?.is_some() {
                            debug!(%blkid, "queueing unchecked block for startup replay");
                            replay_candidates.push(blkid);
                        } else {
                            warn!(
                                %blkid,
                                "unchecked block is indexed but missing block data, skipping startup replay"
                            );
                        }
                        continue;
                    }
                    Ok(Some(BlockStatus::Invalid)) => {
                        debug!(%blkid, "skipping invalid block");
                        continue;
                    }
                    Ok(None) => {
                        warn!(%blkid, "block is indexed but missing status row, skipping");
                        continue;
                    }
                    Err(e) => {
                        error!(%blkid, err = %e, "error loading block status, continuing");
                        continue;
                    }
                }

                // Once we've decided if we want to attach a block, we can
                // continue now.
                if let Some(block) = source.get_ol_block(blkid).await? {
                    if let Err(e) = self.attach_block(
                        block.header().slot(),
                        blkid,
                        *block.header().parent_blkid(),
                    ) {
                        match e {
                            ChainTipError::AttachMissingParent(_, parent_blkid) => {
                                warn!(
                                    %blkid,
                                    %parent_blkid,
                                    "valid block is missing its parent during startup load, skipping"
                                );
                            }
                            err => warn!(%blkid, err = %err, "failed to attach block, continuing"),
                        }
                    }
                } else {
                    warn!(%blkid, "valid block is indexed but missing block data, skipping");
                }
            }

            height += 1;
        }

        Ok(replay_candidates)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use async_trait::async_trait;
    use strata_db_types::{traits::BlockStatus, DbResult};
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };
    use strata_primitives::{Buf32, Buf64, EpochCommitment, OLBlockId};

    use super::{UnfinalizedBlockTracker, UnfinalizedOLBlockSource};

    #[derive(Default)]
    struct TestBlockSource {
        blocks_by_slot: BTreeMap<u64, Vec<OLBlockId>>,
        statuses: HashMap<OLBlockId, BlockStatus>,
        blocks: HashMap<OLBlockId, OLBlock>,
    }

    impl TestBlockSource {
        fn insert_block(&mut self, block: OLBlock, status: Option<BlockStatus>) -> OLBlockId {
            let blkid = block.header().compute_blkid();
            self.blocks_by_slot
                .entry(block.header().slot())
                .or_default()
                .push(blkid);
            self.blocks.insert(blkid, block);
            if let Some(status) = status {
                self.statuses.insert(blkid, status);
            }
            blkid
        }

        fn insert_indexed_id(&mut self, slot: u64, blkid: OLBlockId, status: Option<BlockStatus>) {
            self.blocks_by_slot.entry(slot).or_default().push(blkid);
            if let Some(status) = status {
                self.statuses.insert(blkid, status);
            }
        }
    }

    #[async_trait]
    impl UnfinalizedOLBlockSource for TestBlockSource {
        async fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
            Ok(self.blocks_by_slot.get(&slot).cloned().unwrap_or_default())
        }

        async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>> {
            Ok(self.statuses.get(&blkid).copied())
        }

        async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>> {
            Ok(self.blocks.get(&blkid).cloned())
        }
    }

    fn make_block(slot: u64, parent: OLBlockId, salt: u8) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeader::new(
            1_000 + slot,
            BlockFlags::from(0),
            slot,
            0,
            parent,
            body.compute_hash_commitment(),
            Buf32::from([salt; 32]),
            Buf32::zero(),
        );
        OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
    }

    #[tokio::test]
    async fn loader_surfaces_only_unchecked_blocks_with_data() {
        let genesis_blkid = OLBlockId::from(Buf32::zero());
        let finalized_epoch = EpochCommitment::new(0, 0, genesis_blkid);
        let mut tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
        let mut source = TestBlockSource::default();

        let valid_block = make_block(1, genesis_blkid, 1);
        let valid_blkid = source.insert_block(valid_block, Some(BlockStatus::Valid));

        let unchecked_block = make_block(2, valid_blkid, 2);
        let unchecked_blkid = source.insert_block(unchecked_block, Some(BlockStatus::Unchecked));

        let invalid_block = make_block(2, valid_blkid, 3);
        let invalid_blkid = source.insert_block(invalid_block, Some(BlockStatus::Invalid));

        let missing_status_block = make_block(2, valid_blkid, 4);
        let missing_status_blkid = source.insert_block(missing_status_block, None);

        let missing_data_blkid = OLBlockId::from(Buf32::from([5; 32]));
        source.insert_indexed_id(2, missing_data_blkid, Some(BlockStatus::Unchecked));

        let replay_candidates = tracker
            .load_unfinalized_ol_blocks_async(&source)
            .await
            .expect("loader succeeds");

        assert_eq!(replay_candidates, vec![unchecked_blkid]);
        assert!(tracker.is_seen_block(&valid_blkid));
        assert!(!tracker.is_seen_block(&unchecked_blkid));
        assert!(!tracker.is_seen_block(&invalid_blkid));
        assert!(!tracker.is_seen_block(&missing_status_blkid));
        assert!(!tracker.is_seen_block(&missing_data_blkid));
    }

    #[tokio::test]
    async fn loader_returns_unchecked_replay_candidates_in_slot_scan_order() {
        let genesis_blkid = OLBlockId::from(Buf32::zero());
        let finalized_epoch = EpochCommitment::new(0, 0, genesis_blkid);
        let mut tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
        let mut source = TestBlockSource::default();

        let block1 = make_block(1, genesis_blkid, 1);
        let blkid1 = block1.header().compute_blkid();
        let block2 = make_block(2, blkid1, 2);
        let blkid2 = block2.header().compute_blkid();
        let block3 = make_block(3, blkid2, 3);
        let blkid3 = block3.header().compute_blkid();

        source.insert_block(block3, Some(BlockStatus::Unchecked));
        source.insert_block(block1, Some(BlockStatus::Unchecked));
        source.insert_block(block2, Some(BlockStatus::Unchecked));

        let replay_candidates = tracker
            .load_unfinalized_ol_blocks_async(&source)
            .await
            .expect("loader succeeds");

        assert_eq!(replay_candidates, vec![blkid1, blkid2, blkid3]);
        assert!(!tracker.is_seen_block(&blkid1));
        assert!(!tracker.is_seen_block(&blkid2));
        assert!(!tracker.is_seen_block(&blkid3));
    }

    #[tokio::test]
    async fn loader_skips_valid_block_with_missing_parent() {
        let genesis_blkid = OLBlockId::from(Buf32::zero());
        let finalized_epoch = EpochCommitment::new(0, 0, genesis_blkid);
        let mut tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
        let mut source = TestBlockSource::default();

        let missing_parent = OLBlockId::from(Buf32::from([9; 32]));
        let valid_orphan = make_block(1, missing_parent, 1);
        let valid_orphan_blkid = source.insert_block(valid_orphan, Some(BlockStatus::Valid));

        let replay_candidates = tracker
            .load_unfinalized_ol_blocks_async(&source)
            .await
            .expect("loader succeeds");

        assert!(replay_candidates.is_empty());
        assert!(!tracker.is_seen_block(&valid_orphan_blkid));
    }
}
