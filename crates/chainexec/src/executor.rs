//! Chain executor.

use strata_chaintsn::{context::StateAccessor, transition::process_block};
use strata_primitives::prelude::*;
use strata_state::{block::L2BlockBundle, header::L2Header, state_op::StateCache};
use tracing::*;

use crate::{BlockExecContext, BlockExecutionOutput, Error, ExecContext, ExecResult, State};

pub struct ChainExecutor {
    params: RollupParams,
}

impl<C: ExecContext> ChainExecutor<C> {
    pub fn new(params: RollupParams) -> Self {
        Self { params }
    }

    /// Tries to process a block.  This only works if it's a next block after
    /// the current tip block.
    pub fn try_process_block(
        &self,
        blkid: &L2BlockId,
        block: &L2BlockBundle,
        state: &mut impl StateAccessor,
    ) -> ExecResult<BlockExecutionOutput> {
        Err(Error::Unimplemented)
    }
}

fn try_process_block(
    blkid: &L2BlockId,
    block: &L2BlockBundle,
    params: &RollupParams,
    ctx: &impl ExecContext,
) -> ExecResult<()> {
    let pre_state = ctx.fetch_block_toplevel_post_state(blkid)?;

    let header = block.header();
    let body = block.body();

    // Get the prev epoch to check if the epoch advanced, and the prev
    // epoch's terminal in case we need it.
    let pre_state_epoch_finishing = pre_state.is_epoch_finishing();
    let pre_state_epoch = pre_state.cur_epoch();

    // Apply the state transition.
    let mut pre_cache = StateCache::new(pre_state);
    process_block(&mut pre_cache, header, body, params)?;

    // Finalize the post state.
    let wb = pre_cache.finalize();
    let post_state = wb.new_toplevel_state();
    let post_state_epoch = post_state.cur_epoch();

    // Sanity check.
    assert!(
        (!pre_state_epoch_finishing && post_state_epoch == pre_state_epoch)
            || (pre_state_epoch_finishing && post_state_epoch == pre_state_epoch + 1),
        "chainexec: nonsensical post-state epoch (pre={pre_state_epoch}, post={post_state_epoch})"
    );

    // Verify state root matches.
    let computed_sr = post_state.compute_state_root();
    if *header.state_root() != computed_sr {
        warn!(block_sr = %header.state_root(), %computed_sr, "state root mismatch");
        Err(Error::StateRootMismatch)?
    }

    // TODO copy most of this from handle_new_block in FCM
    Err(Error::Unimplemented)
}
