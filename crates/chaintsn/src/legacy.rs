//! Legacy routines extracted from `StateCache`.

use strata_bridge_types::DepositIntent;
use strata_identifiers::{AccountSerial, L1BlockCommitment};
use strata_ol_chainstate_types::Chainstate;

use crate::context::StateAccessor;

#[derive(Debug)]
pub struct FauxStateCache<'s, S> {
    state: &'s mut S,
}

impl<'s, S: StateAccessor> FauxStateCache<'s, S> {
    pub fn new(state: &'s mut S) -> Self {
        Self { state }
    }

    pub fn inner(&self) -> &S {
        self.state
    }

    pub fn inner_mut(&mut self) -> &mut S {
        self.state
    }

    pub fn state(&self) -> &Chainstate {
        self.state.state_untracked()
    }

    fn state_mut(&mut self) -> &mut Chainstate {
        self.state.state_mut_untracked()
    }

    /// Update HeaderVerificationState
    pub fn update_verified_blk(&mut self, blk: L1BlockCommitment) {
        self.state_mut().l1_view_mut().update_verified_blk(blk);
    }

    /// Writes a deposit intent into an execution environment's input queue.
    pub fn insert_deposit_intent(&mut self, ee_id: AccountSerial, intent: DepositIntent) {
        assert_eq!(
            ee_id,
            AccountSerial::zero(),
            "stateop: only support execution env 0 right now"
        );
        self.state_mut()
            .exec_env_state_mut()
            .pending_deposits
            .push_back(intent);
    }

    /// Remove a deposit intent from the pending deposits queue.
    ///
    /// This actually removes possibly multiple deposit intents.
    pub fn consume_deposit_intent(&mut self, idx: u64) {
        let deposits = self.state_mut().exec_env_state_mut().pending_deposits_mut();

        let front_idx = deposits
            .front_idx()
            .expect("stateop: empty deposit intent queue");

        // deposit intent indices processed sequentially, without any gaps
        let to_drop_count = idx
            .checked_sub(front_idx) // ensures to_drop_idx >= front_idx
            .expect("stateop: unable to consume deposit intent")
            + 1;

        deposits
            .pop_front_n_vec(to_drop_count as usize) // ensures to_drop_idx < front_idx + len
            .expect("stateop: unable to consume deposit intent");
    }
}
