//! State accessor impls.

use std::sync::Arc;

use strata_chaintsn::context::StateAccessor;
use strata_primitives::prelude::*;
use strata_state::{chain_state::Chainstate, state_op::StateCache};
use strata_storage::ChainstateManager;

#[expect(dead_code, reason = "Some inner types don't have Debug impls")]
pub(crate) struct WbStateAccessorImpl {
    /// Chainstate manager to fetch "deep" information we might not have in memory.
    // we aren't actually using this yet, but we will when we have accounts
    chs_man: Arc<ChainstateManager>,

    /// Current toplevel state we can write to at-will.
    // this uses state cache for legacy compatibility reasons, will be replaced
    // eventually
    toplevel_chs_cache: StateCache,
}

impl StateAccessor for WbStateAccessorImpl {
    fn state_untracked(&self) -> &Chainstate {
        self.toplevel_chs_cache.state()
    }

    fn state_mut_untracked(&mut self) -> &mut Chainstate {
        self.toplevel_chs_cache.state_mut()
    }

    fn slot(&self) -> u64 {
        self.toplevel_chs_cache.state().chain_tip_slot()
    }

    fn set_slot(&mut self, slot: u64) {
        self.toplevel_chs_cache.set_slot(slot);
    }

    fn prev_block(&self) -> L2BlockCommitment {
        *self.toplevel_chs_cache.state().prev_block()
    }

    fn set_prev_block(&mut self, block: L2BlockCommitment) {
        self.toplevel_chs_cache.set_prev_block(block);
    }

    fn cur_epoch(&self) -> u64 {
        self.toplevel_chs_cache.state().cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u64) {
        self.toplevel_chs_cache.set_cur_epoch(epoch);
    }

    fn prev_epoch(&self) -> EpochCommitment {
        *self.toplevel_chs_cache.state().prev_epoch()
    }

    fn set_prev_epoch(&mut self, epoch: EpochCommitment) {
        self.toplevel_chs_cache.set_prev_epoch(epoch);
    }

    fn finalized_epoch(&self) -> EpochCommitment {
        *self.toplevel_chs_cache.state().finalized_epoch()
    }

    fn set_finalized_epoch(&mut self, epoch: EpochCommitment) {
        self.toplevel_chs_cache.set_finalized_epoch(epoch);
    }

    fn last_l1_block(&self) -> L1BlockCommitment {
        self.toplevel_chs_cache.state().l1_view().get_safe_block()
    }

    fn epoch_finishing_flag(&self) -> bool {
        self.toplevel_chs_cache.state().is_epoch_finishing()
    }

    fn set_epoch_finishing_flag(&mut self, flag: bool) {
        self.toplevel_chs_cache.set_epoch_finishing_flag(flag);
    }
}
