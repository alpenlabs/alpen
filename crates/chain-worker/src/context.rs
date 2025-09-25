//! Chain executor context impls.

use strata_chainexec::{Error as ExecError, ExecContext};
use strata_ol_chain_types::L2BlockHeader;
use strata_primitives::prelude::*;
use strata_state::chain_state::Chainstate;

use crate::{WorkerContext, WorkerError};

#[derive(Debug)]
pub(crate) struct WorkerExecCtxImpl<'c, W> {
    pub worker_context: &'c W,
}

impl<'c, W: WorkerContext> ExecContext for WorkerExecCtxImpl<'c, W> {
    type Error = WorkerError;

    fn fetch_l2_header(&self, blkid: &L2BlockId) -> Result<L2BlockHeader, ExecError<Self::Error>> {
        match self.worker_context.fetch_header(blkid) {
            Ok(Some(header)) => Ok(header),
            Ok(None) => Err(ExecError::Context(WorkerError::MissingL2Block(*blkid))),
            Err(err) => Err(ExecError::Context(err)),
        }
    }

    fn fetch_block_toplevel_post_state(
        &self,
        blkid: &L2BlockId,
    ) -> Result<Chainstate, ExecError<Self::Error>> {
        // This impl might be suboptimal, should we do real reconstruction?
        //
        // Maybe actually make this return a `StateAccessor` already?
        match self.worker_context.fetch_block_write_batch(blkid) {
            Ok(Some(wb)) => Ok(wb.into_toplevel()),
            Ok(None) => Err(ExecError::Context(WorkerError::MissingWriteBatch(*blkid))),
            Err(err) => Err(ExecError::Context(err)),
        }
    }
}
