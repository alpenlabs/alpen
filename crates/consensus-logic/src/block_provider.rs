use std::error::Error;

use strata_db_types::{traits::BlockStatus, DbError};
use strata_identifiers::Slot;
use strata_primitives::OLBlockId;
use strata_storage::{L2BlockManager, OLBlockManager};

#[derive(Clone, Debug)]
pub struct BlockMeta {
    slot: Slot,
    blkid: OLBlockId,
    parent_blkid: OLBlockId,
}

impl BlockMeta {
    pub(crate) fn new(slot: Slot, blkid: OLBlockId, parent_blkid: OLBlockId) -> Self {
        Self {
            slot,
            blkid,
            parent_blkid,
        }
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn blkid(&self) -> &OLBlockId {
        &self.blkid
    }

    pub fn parent_blkid(&self) -> OLBlockId {
        self.parent_blkid
    }
}

pub trait BlockProvider {
    type Error: Error + Send + Sync + 'static;
    fn get_blocks_at_height(&self, height: Slot) -> Result<Vec<OLBlockId>, Self::Error>;
    fn get_block_status(&self, blkid: &OLBlockId) -> Result<Option<BlockStatus>, Self::Error>;
    fn get_block_data(&self, blkid: &OLBlockId) -> Result<Option<BlockMeta>, Self::Error>;
}

impl BlockProvider for L2BlockManager {
    type Error = DbError;

    fn get_blocks_at_height(&self, height: Slot) -> Result<Vec<OLBlockId>, Self::Error> {
        self.get_blocks_at_height_blocking(height)
    }

    fn get_block_status(&self, blkid: &OLBlockId) -> Result<Option<BlockStatus>, Self::Error> {
        self.get_block_status_blocking(blkid)
    }

    fn get_block_data(&self, blkid: &OLBlockId) -> Result<Option<BlockMeta>, Self::Error> {
        let bundle = self.get_block_data_blocking(blkid)?;
        Ok(bundle.map(|b| {
            let slot = b.header().header().slot();
            let parent_blkid = b.header().header().prev_block();
            BlockMeta::new(slot, *blkid, parent_blkid)
        }))
    }
}

impl BlockProvider for OLBlockManager {
    type Error = DbError;

    fn get_blocks_at_height(&self, height: Slot) -> Result<Vec<OLBlockId>, Self::Error> {
        self.get_blocks_at_height_blocking(height)
    }

    fn get_block_status(&self, blkid: &OLBlockId) -> Result<Option<BlockStatus>, Self::Error> {
        self.get_block_status_blocking(*blkid)
    }

    fn get_block_data(&self, blkid: &OLBlockId) -> Result<Option<BlockMeta>, Self::Error> {
        let block = self.get_block_data_blocking(*blkid)?;
        Ok(block.map(|b| {
            let slot = b.header().slot();
            let parent_blkid = b.header().parent_blkid();
            BlockMeta::new(slot, *blkid, *parent_blkid)
        }))
    }
}
