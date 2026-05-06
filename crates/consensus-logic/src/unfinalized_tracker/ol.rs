use async_trait::async_trait;
use strata_db_types::{traits::BlockStatus, DbResult};
use strata_identifiers::Slot;
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::OLBlockId;
use strata_storage::OLBlockManager;
use tracing::{debug, error, warn};

use super::UnfinalizedBlockTracker;

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
    ) -> anyhow::Result<()> {
        let mut height = self.finalized_epoch().last_slot() + 1;

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
                //
                // TODO(STR-3370): if a block doesn't have a concrete status (either
                // missing or explicit unchecked) should we put it into a queue
                // to be processed?
                match source.get_block_status(blkid).await {
                    Ok(Some(status)) => {
                        if status != BlockStatus::Valid {
                            debug!(%blkid, "skipping attaching block not known to be valid");
                            continue;
                        }
                    }
                    Ok(_) => {
                        debug!(%blkid, "block status not available, will check later");
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
                        warn!(%blkid, err = %e, "failed to attach block, continuing");
                    }
                } else {
                    error!(%blkid, "missing expected block from database!  wtf?");
                }
            }

            height += 1;
        }

        Ok(())
    }
}
