//! Execution context traits.

use std::collections::HashMap;

use strata_primitives::prelude::*;
use strata_state::{chain_state::Chainstate, prelude::*};
use thiserror::Error;

use crate::Error as ExecError;

/// External context the block executor needs to operate.
pub trait ExecContext {
    /// The error type for context operations
    type Error: std::error::Error + Send + Sync + 'static;

    /// Fetches an L2 block's header.
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> Result<L2BlockHeader, ExecError<Self::Error>>;

    /// Fetches a block's toplevel post-state.
    fn fetch_block_toplevel_post_state(
        &self,
        blkid: &L2BlockId,
    ) -> Result<Chainstate, ExecError<Self::Error>>;

    // TODO L1 manifests
}

#[derive(Debug, Error)]
pub enum MemExecContextError {
    #[error("missing L2 header {0}")]
    MissingL2Header(L2BlockId),

    #[error("missing block post-state {0}")]
    MissingBlockPostState(L2BlockId),
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
    type Error = MemExecContextError;

    fn fetch_l2_header(&self, blkid: &L2BlockId) -> Result<L2BlockHeader, ExecError<Self::Error>> {
        self.headers.get(blkid).cloned().ok_or(ExecError::Context(
            MemExecContextError::MissingL2Header(*blkid),
        ))
    }

    fn fetch_block_toplevel_post_state(
        &self,
        blkid: &L2BlockId,
    ) -> Result<Chainstate, ExecError<Self::Error>> {
        self.chainstates
            .get(blkid)
            .cloned()
            .ok_or(ExecError::Context(
                MemExecContextError::MissingBlockPostState(*blkid),
            ))
    }
}
