//! Execution context traits.

use strata_primitives::prelude::*;
use strata_state::{block::L2BlockBundle, chain_state::Chainstate, prelude::*};

use crate::ExecResult;

/// External context the block executor needs to operate.
pub trait ExecContext {
    /// Fetches an L2 block's header.
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader>;

    /// Fetches a block's toplevel post-state.
    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate>;

    // TODO L1 manifests
}

pub trait BlockContext {
    // TODO
}
