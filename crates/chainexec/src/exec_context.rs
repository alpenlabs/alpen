//! Execution context traits.

use std::collections::HashMap;

use strata_primitives::prelude::*;
use strata_state::{chain_state::Chainstate, prelude::*};

use crate::ExecResult;

/// External context the block executor needs to operate.
pub trait ExecContext {
    /// Fetches an L2 block's header.
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader>;

    /// Fetches a block's toplevel post-state.
    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate>;

    // TODO L1 manifests
}

#[derive(Debug, Clone, Default)]
pub struct MemExecContext {
    headers: HashMap<L2BlockId, L2BlockHeader>,
    chainstates: HashMap<L2BlockId, Chainstate>,
}

impl MemExecContext {
    pub fn put_header(&mut self, blkid: L2BlockId, header: L2BlockHeader) {
        self.headers.insert(blkid, header);
    }

    pub fn put_chainstate(&mut self, blkid: L2BlockId, chainstate: Chainstate) {
        self.chainstates.insert(blkid, chainstate);
    }
}

impl ExecContext for MemExecContext {
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader> {
        self.headers
            .get(blkid)
            .cloned()
            .ok_or(crate::Error::MissingL2Header(*blkid))
    }

    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate> {
        self.chainstates
            .get(blkid)
            .cloned()
            .ok_or(crate::Error::MissingBlockPostState(*blkid))
    }
}
