//! Chain executor context impls.

use strata_chainexec::{ExecContext, ExecResult};
use strata_primitives::prelude::*;
use strata_state::{chain_state::Chainstate, prelude::L2BlockHeader};

use crate::WorkerContext;

#[derive(Debug)]
pub(crate) struct WorkerExecCtxImpl<'c, W> {
    pub worker_context: &'c W,
}

impl<'c, W: WorkerContext> ExecContext for WorkerExecCtxImpl<'c, W> {
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader> {
        self.worker_context
            .fetch_header(blkid)?
            .ok_or(strata_chainexec::Error::MissingL2Header(*blkid))
    }

    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate> {
        // This impl might be suboptimal, should we do real reconstruction?
        //
        // Maybe actually make this return a `StateAccessor` already?
        let wb = self
            .worker_context
            .fetch_block_write_batch(blkid)?
            .ok_or(strata_chainexec::Error::MissingBlockPostState(*blkid))?;
        Ok(wb.into_toplevel())
    }
}
