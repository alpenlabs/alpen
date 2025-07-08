use std::sync::Arc;

use strata_db::DbError;
use strata_eectl::{
    errors::EngineResult,
    messages::ExecPayloadData,
    worker::{ExecEnvId, ExecWorkerContext},
};
use strata_primitives::l2::L2BlockCommitment;
use strata_state::{block::L2BlockBundle, header::L2Header, id::L2BlockId};
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

    fn get_cur_tip(&self) -> EngineResult<L2BlockCommitment> {
        let blkid = self.l2man.get_tip_block_blocking()?;
        let slot = self
            .l2man
            .get_block_data_blocking(&blkid)?
            .ok_or(DbError::MissingL2Block(blkid))?
            .header()
            .slot();
        Ok(L2BlockCommitment::new(slot, blkid))
    }

    fn get_blkid_at_height(&self, height: u64) -> EngineResult<L2BlockId> {
        Ok(self
            .l2man
            .get_blocks_at_height_blocking(height)?
            .first()
            .cloned()
            .ok_or(DbError::MissingL2BlockHeight(height))?)
    }

    fn get_block(&self, blkid: L2BlockId) -> EngineResult<L2BlockBundle> {
        Ok(self
            .l2man
            .get_block_data_blocking(&blkid)?
            .ok_or(DbError::MissingL2Block(blkid))?)
    }
}
