//! Impl of `StateAccessor` type hierarchy.
//!
//! This includes:
//! * fully in-memory impl we can use in proofs and testing
//! * planned: wrapper impl that generates state diffs along the way we can apply later
//! * planned: database-backed impl that will make queries against the database for non-toplevel
//!   data

use strata_chaintsn::context::StateAccessor;
use strata_ol_chainstate_types::{Chainstate, StateCache, WriteBatch};
use strata_primitives::prelude::*;

/// Accessor for state in memory.
#[derive(Debug)]
pub struct MemStateAccessor {
    state_cache: StateCache,
}

impl MemStateAccessor {
    pub fn new(chainstate: Chainstate) -> Self {
        Self {
            state_cache: StateCache::new(chainstate),
        }
    }

    /// Constructs a write batch out of the state changes we've written.
    pub fn into_write_batch(self) -> WriteBatch {
        self.state_cache.finalize()
    }
}

impl StateAccessor for MemStateAccessor {
    fn state_untracked(&self) -> &Chainstate {
        self.state_cache.state()
    }

    fn state_mut_untracked(&mut self) -> &mut Chainstate {
        self.state_cache.state_mut()
    }

    fn slot(&self) -> u64 {
        self.state_cache.state().chain_tip_slot()
    }

    fn set_slot(&mut self, slot: u64) {
        self.state_cache.set_slot(slot);
    }

    fn prev_block(&self) -> L2BlockCommitment {
        *self.state_cache.state().prev_block()
    }

    fn set_prev_block(&mut self, block: L2BlockCommitment) {
        self.state_cache.set_prev_block(block);
    }

    fn cur_epoch(&self) -> u64 {
        self.state_cache.state().cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u64) {
        self.state_cache.set_cur_epoch(epoch);
    }

    fn prev_epoch(&self) -> EpochCommitment {
        *self.state_cache.state().prev_epoch()
    }

    fn set_prev_epoch(&mut self, epoch: EpochCommitment) {
        self.state_cache.set_prev_epoch(epoch);
    }

    fn finalized_epoch(&self) -> EpochCommitment {
        *self.state_cache.state().finalized_epoch()
    }

    fn set_finalized_epoch(&mut self, epoch: EpochCommitment) {
        self.state_cache.set_finalized_epoch(epoch);
    }

    fn last_l1_block(&self) -> L1BlockCommitment {
        let l1_view = self.state_cache.state().l1_view();
        L1BlockCommitment::from_height_u64(l1_view.safe_height(), *l1_view.safe_blkid())
            .expect("height should be valid")
    }

    fn epoch_finishing_flag(&self) -> bool {
        self.state_cache.state().is_epoch_finishing()
    }

    fn set_epoch_finishing_flag(&mut self, flag: bool) {
        self.state_cache.set_epoch_finishing_flag(flag);
    }
}
