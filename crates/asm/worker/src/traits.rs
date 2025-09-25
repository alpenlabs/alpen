//! Traits for the chain worker to interface with the underlying system.

use bitcoin::{Block, Network};
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;

use crate::WorkerResult;

/// Context trait for a worker to interact with the database and Bitcoin Client.
pub trait WorkerContext {
    /// Fetches a Bitcoin [`Block`] at a given height.
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block>;

    /// Fetches the [`AsmState`] given the block id.
    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState>;

    /// Fetches the latest [`AsmState`] - the one that corresponds to the "highest" block.
    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>>;

    /// Puts the [`AsmState`] into DB.
    fn store_anchor_state(&self, blockid: &L1BlockCommitment, state: &AsmState)
    -> WorkerResult<()>;

    /// A Bitcoin network identifier.
    fn get_network(&self) -> WorkerResult<Network>;
}
