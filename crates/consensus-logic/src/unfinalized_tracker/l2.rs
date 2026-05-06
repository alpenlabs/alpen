#![expect(deprecated, reason = "legacy old code is retained for compatibility")]

use strata_db_types::traits::BlockStatus;
use strata_ol_chain_types::L2Header;
use strata_storage::L2BlockManager;
use tracing::{debug, error, warn};

use super::UnfinalizedBlockTracker;

impl UnfinalizedBlockTracker {
    /// Loads the unfinalized blocks into the tracker which are already in the DB.
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub fn load_unfinalized_blocks(&mut self, l2_blk_mgr: &L2BlockManager) -> anyhow::Result<()> {
        let mut height = self.finalized_epoch().last_slot() + 1;

        loop {
            let blkids = match l2_blk_mgr.get_blocks_at_height_blocking(height) {
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
                match l2_blk_mgr.get_block_status_blocking(&blkid) {
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
                if let Some(block) = l2_blk_mgr.get_block_data_blocking(&blkid)? {
                    if let Err(e) = self.attach_block(
                        block.header().header().slot(),
                        blkid,
                        *block.header().header().parent(),
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
