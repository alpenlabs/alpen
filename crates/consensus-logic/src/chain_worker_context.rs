//! Context impl to instantiate chain worker with.

use std::sync::Arc;

use strata_chain_worker::*;
use strata_chainexec::{BlockExecutionOutput, ChangedState};
use strata_db::DbError;
use strata_primitives::prelude::*;
use strata_state::{batch::EpochSummary, block::L2BlockBundle, prelude::*};
use strata_storage::{ChainstateManager, CheckpointDbManager, L2BlockManager};
use tracing::*;

pub struct ChainWorkerCtx {
    l2man: Arc<L2BlockManager>,
    chsman: Arc<ChainstateManager>,
    ckman: Arc<CheckpointDbManager>,
}

impl ChainWorkerCtx {
    pub fn new() -> Self {
        todo!()
    }
}

impl WorkerContext for ChainWorkerCtx {
    fn fetch_block(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockBundle>> {
        Ok(self
            .l2man
            .get_block_data_blocking(blkid)
            .map_err(conv_db_err)?)
    }

    fn fetch_header(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockHeader>> {
        // FIXME make this only fetch the header
        Ok(self
            .l2man
            .get_block_data_blocking(blkid)
            .map_err(conv_db_err)?
            .map(|b| b.header().header().clone()))
    }

    fn fetch_block_output(&self, blkid: &L2BlockId) -> WorkerResult<Option<BlockExecutionOutput>> {
        let Some(chs_entry) = self.chsman.get_toplevel_chainstate_blocking(blkid)? else {
            return Ok(None);
        };

        // Construct a block exec output on the fly for this.
        let tl_chs = chs_entry.to_chainstate();
        let sr = tl_chs.compute_state_root();
        let logs = Vec::new();
        let changed_state = ChangedState::new(tl_chs);

        Ok(Some(BlockExecutionOutput::new(sr, logs, changed_state)))
    }

    fn store_block_output(
        &self,
        blkid: &L2BlockId,
        output: BlockExecutionOutput,
    ) -> WorkerResult<()> {
        // TODO we really do have to change how the database works to implement
        // this, don't we?
        todo!()
    }

    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>> {
        let epochs = self
            .ckman
            .get_epoch_commitments_at_blocking(epoch as u64)
            .map_err(conv_db_err)?;

        let mut summaries = Vec::new();
        for epoch in epochs {
            let Some(s) = self
                .ckman
                .get_epoch_summary_blocking(epoch)
                .map_err(conv_db_err)?
            else {
                warn!(?epoch, "found epoch commitment but missing summary");
                continue;
            };

            summaries.push(s);
        }

        Ok(summaries)
    }

    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary> {
        Ok(self
            .ckman
            .get_epoch_summary_blocking(epoch)
            .map_err(conv_db_err)?
            .ok_or_else(|| WorkerError::MissingEpochSummary(*epoch))?)
    }

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        self.ckman.insert_epoch_summary_blocking(summary)?;
        Ok(())
    }
}

fn conv_db_err(e: DbError) -> WorkerError {
    // TODO fixme
    WorkerError::Unimplemented
}
