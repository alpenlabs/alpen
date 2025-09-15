//! Traits for the chain worker to interface with the underlying system.

use bitcoin::{Block, Network};
use strata_asm_common::AnchorState;
use strata_primitives::prelude::*;

use crate::WorkerResult;

/// Context trait for a worker to interact with the database and network.
pub trait WorkerContext {
    /// Fetches a whole btc [`Block`].
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block>;

    /// Fetches the [`AnchorState`] given the block id.
    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AnchorState>;

    /// Fetches the latest [`AnchorState`] - the one that corresponds to the "highest" block.
    fn get_latest_anchor_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AnchorState)>>;

    /// Puts the [`AnchorState`] into DB.
    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &AnchorState,
    ) -> WorkerResult<()>;

    /// A Bitcoin network identifier.
    fn get_network(&self) -> WorkerResult<Network>;
}
