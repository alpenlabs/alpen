//! Traits for the chain worker to interface with the underlying system.

use std::sync::Arc;

use strata_chainexec::{BlockExecutionOutput, CheckinExecutionOutput};
use strata_primitives::{batch::EpochSummary, prelude::*};
use strata_state::{
    block::L2BlockBundle, chain_state::Chainstate, header::L2BlockHeader, state_op::WriteBatch,
};

use crate::WorkerResult;

/// Context trait for a worker to interact with the database.
pub trait WorkerContext {
    // Chain access functions

    /// Fetches a whole block bundle.
    fn fetch_block(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockBundle>>;

    /// Fetches a block's header.
    fn fetch_header(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockHeader>>;

    /// Stores an epoch summary in the database.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;

    /// Fetches a specific epoch summary.
    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary>;

    /// Fetches all summaries for an epoch index.
    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>>;

    // State access functions

    /// Stores a block execution's output.  This MAY be broken up into multiple
    /// separate pieces.
    fn store_block_output(
        &self,
        blkid: &L2BlockId,
        output: &BlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Stores a check in execution's output.  This MAY be broken up into
    /// multiple separate pieces.
    fn store_checkin_output(
        &self,
        epoch: &EpochCommitment,
        output: &CheckinExecutionOutput,
    ) -> WorkerResult<()>;

    /// Fetches a block's write batch.
    fn fetch_block_write_batch(&self, blkid: &L2BlockId) -> WorkerResult<Option<WriteBatch>>;

    /// Gets the finalized toplevel state.
    fn get_finalized_toplevel_state(&self) -> WorkerResult<Arc<Chainstate>>;

    /// Merges write batches up to the given epoch's state into the finalized
    /// state we accept.  This means we have to load fewer write batches.
    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()>;
}
