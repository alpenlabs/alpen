use std::sync::Arc;

use strata_db::DbError;
use strata_eectl::{
    errors::EngineResult,
    messages::ExecPayloadData,
    worker::{ExecEnvId, ExecWorkerContext},
};
use strata_primitives::l2::L2BlockCommitment;
use strata_state::{header::L2Header, id::L2BlockId};
use strata_storage::L2BlockManager;

#[expect(missing_debug_implementations)]
pub struct ExecWorkerCtx {
    l2man: Arc<L2BlockManager>,
}

impl ExecWorkerCtx {
    pub fn new(l2man: Arc<L2BlockManager>) -> Self {
        Self { l2man }
    }
}

impl ExecWorkerContext for ExecWorkerCtx {
    fn fetch_exec_payload(
        &self,
        block: &L2BlockCommitment,
        _eeid: &ExecEnvId,
    ) -> EngineResult<Option<ExecPayloadData>> {
        let blkid = block.blkid();
        let bundle = self.l2man.get_block_data_blocking(blkid)?;

        match bundle {
            Some(bundle) => Ok(Some(ExecPayloadData::from_l2_block_bundle(&bundle))),
            None => Ok(None),
        }
    }

    fn fetch_parent(&self, block: &L2BlockCommitment) -> EngineResult<L2BlockCommitment> {
        let parent_blk = self
            .l2man
            .get_block_data_blocking(block.blkid())?
            .ok_or(DbError::MissingL2Block(*block.blkid()))?;
        let parent_blkid = parent_blk.header().parent();
        // REVIEW: Check if simply doing -1 is enough or we want to fetch the parent block again and
        // get the slot from there in case the slot are empty
        let parent_slot = block.slot() - 1;
        Ok(L2BlockCommitment::new(parent_slot, *parent_blkid))
    }

    fn fetch_cur_tip(&self) -> EngineResult<L2BlockCommitment> {
        let blkid = self.l2man.get_tip_block_blocking()?;
        let slot = self
            .l2man
            .get_block_data_blocking(&blkid)?
            .ok_or(DbError::MissingL2Block(blkid))?
            .header()
            .slot();
        Ok(L2BlockCommitment::new(slot, blkid))
    }

    fn fetch_blkid_at_height(&self, height: u64) -> EngineResult<Option<L2BlockId>> {
        Ok(self
            .l2man
            .get_blocks_at_height_blocking(height)?
            .first()
            .cloned())
    }
}
