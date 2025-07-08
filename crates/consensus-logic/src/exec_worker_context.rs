use std::sync::Arc;

use strata_eectl::{
    errors::EngineResult,
    messages::ExecPayloadData,
    worker::{ExecEnvId, ExecWorkerContext},
};
use strata_primitives::l2::L2BlockCommitment;
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
}
