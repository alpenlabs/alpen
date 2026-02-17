use std::collections::*;

use parking_lot::Mutex;
use strata_ol_chain_types::{L2BlockBundle, L2BlockId, L2Header};

use crate::{
    traits::{BlockStatus, *},
    DbError, DbResult,
};

/// Dummy implementation that isn't really compliant with the spec, but we don't
/// care because we just want to get something running. :sunglasses:.
#[derive(Debug)]
pub struct StubL2Db {
    blocks: Mutex<HashMap<L2BlockId, L2BlockBundle>>,
    statuses: Mutex<HashMap<L2BlockId, BlockStatus>>,
    heights: Mutex<HashMap<u64, Vec<L2BlockId>>>,
}

impl Default for StubL2Db {
    fn default() -> Self {
        Self::new()
    }
}

impl StubL2Db {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
            statuses: Mutex::new(HashMap::new()),
            heights: Mutex::new(HashMap::new()),
        }
    }
}

#[expect(
    deprecated,
    reason = "legacy L2 stub implementation is retained for compatibility"
)]
impl L2BlockDatabase for StubL2Db {
    fn put_block_data(&self, bundle: L2BlockBundle) -> DbResult<()> {
        let blkid = bundle.block().header().get_blockid();
        let idx = bundle.block().header().slot();

        {
            let mut tbl = self.blocks.lock();
            tbl.insert(blkid, bundle);
        }

        {
            let mut tbl = self.heights.lock();
            tbl.entry(idx).or_default().push(blkid);
        }

        Ok(())
    }

    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool> {
        // Remove from blocks, capturing the bundle to compute its height
        let maybe_bundle = {
            let mut blocks_tbl = self.blocks.lock();
            blocks_tbl.remove(&id)
        };

        let Some(bundle) = maybe_bundle else {
            return Ok(false);
        };

        // Remove id from heights[slot]
        let slot = bundle.block().header().slot();
        {
            let mut heights_tbl = self.heights.lock();
            if let Some(vec_ids) = heights_tbl.get_mut(&slot) {
                vec_ids.retain(|&block_id| block_id != id);
            }
        }

        // Remove status for this id, if any
        {
            let mut statuses_tbl = self.statuses.lock();
            statuses_tbl.remove(&id);
        }

        Ok(true)
    }

    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()> {
        let mut tbl = self.statuses.lock();
        tbl.insert(id, status);
        Ok(())
    }

    fn get_block_data(&self, id: L2BlockId) -> DbResult<Option<L2BlockBundle>> {
        let tbl = self.blocks.lock();
        Ok(tbl.get(&id).cloned())
    }

    fn get_blocks_at_height(&self, idx: u64) -> DbResult<Vec<L2BlockId>> {
        let tbl = self.heights.lock();
        Ok(tbl.get(&idx).cloned().unwrap_or_default())
    }

    fn get_block_status(&self, id: L2BlockId) -> DbResult<Option<BlockStatus>> {
        let tbl = self.statuses.lock();
        Ok(tbl.get(&id).cloned())
    }

    fn get_tip_block(&self) -> DbResult<L2BlockId> {
        let tbl = self.heights.lock();
        let max_height = tbl.keys().max().cloned();
        if let Some(height) = max_height {
            if let Some(blocks) = tbl.get(&height) {
                match blocks.first().cloned() {
                    Some(block_id) => return Ok(block_id),
                    None => return Err(DbError::NotBootstrapped),
                }
            }
        }
        Err(DbError::NotBootstrapped)
    }
}
