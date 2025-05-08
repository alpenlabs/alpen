//! Chain executor.

use strata_primitives::prelude::*;
use strata_state::block::L2BlockBundle;

use crate::{Error, ExecContext, ExecResult};

pub struct ChainExecutor<C: ExecContext> {
    context: C,
    params: RollupParams,
}

impl<C: ExecContext> ChainExecutor<C> {
    pub fn new(context: C, params: RollupParams) -> Self {
        Self { context, params }
    }

    /// Tries to process a block.  This only works if it's a next block after
    /// the current tip block.
    pub fn try_process_block(
        &self,
        blkid: &L2BlockId,
        block: &L2BlockBundle,
        ctx: &mut impl BlockStateContext,
    ) -> ExecResult<()> {
        // TODO copy most of this from handle_new_block in FCM
        Err(Error::Unimplemented)
    }
}
