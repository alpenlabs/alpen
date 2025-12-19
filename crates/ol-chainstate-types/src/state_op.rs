//! Low-level operations we can make to write to chain state.
//!
//! This currently only can manipulate the toplevel chain state, but we might
//! decide to expand the chain state in the future such that we can't keep it
//! entire in memory.

use bitcoin::block::Header;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_types::L1VerificationError;
use strata_bridge_types::DepositIntent;
use strata_identifiers::Epoch;
use strata_primitives::{
    epoch::EpochCommitment,
    l2::{L2BlockCommitment, L2BlockId},
};

use crate::{Chainstate, ChainstateEntry};

/// Collection of writes we're making to the state.
#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct WriteBatch {
    /// Full "toplevel" state.
    new_toplevel_state: Chainstate,
}

impl WriteBatch {
    /// Creates a new instance from the toplevel state and a list of ops.
    pub fn new(new_toplevel_state: Chainstate) -> Self {
        Self { new_toplevel_state }
    }

    /// Creates a new instance from the new toplevel state and assumes no
    /// changes to the bulk state.
    pub fn new_replace_toplevel(new_state: Chainstate) -> Self {
        Self::new(new_state)
    }

    pub fn new_toplevel_state(&self) -> &Chainstate {
        &self.new_toplevel_state
    }

    /// Extracts the toplevel state, discarding the write ops.
    pub fn into_toplevel(self) -> Chainstate {
        self.new_toplevel_state
    }
}

// TODO reversiblity stuff?

/// On a given in-memory chainstate, applies a write batch.
///
/// This must succeed.  Pancis if it does not.
pub fn apply_write_batch_to_chainstate(_chainstate: Chainstate, batch: &WriteBatch) -> Chainstate {
    // This replaces the whole toplevel state.  This probably makes you think
    // it doesn't make sense to take the chainstate arg at all.  But this will
    // probably make more sense in the future when we make the state structure
    // more sophisticated, splitting apart the epoch state from the per-slot
    // state more, and also the bulk state.
    //
    // Since the only state op possible is `Noop`, we can just ignore them all
    // without even iterating over them.
    batch.new_toplevel_state.clone()
}

/// Cache that writes to state and remembers the series of operations made to it
/// so they can be persisted to disk without saving the chainstate.
///
/// If we ever have a large state that's persisted to disk, this will eventually
/// be made generic over a state provider that exposes access to that and then
/// the `WriteBatch` will include writes that can be made to that.
#[derive(Debug)]
pub struct StateCache {
    /// Original toplevel state that we started from, in case we need to reference it.
    original_state: Chainstate,

    /// New state that we're modifying.
    new_state: Chainstate,
}

impl StateCache {
    pub fn new(state: Chainstate) -> Self {
        Self {
            original_state: state.clone(),
            new_state: state,
        }
    }

    // Basic accessors.

    pub fn state(&self) -> &Chainstate {
        &self.new_state
    }

    pub fn state_mut(&mut self) -> &mut Chainstate {
        &mut self.new_state
    }

    pub fn original_state(&self) -> &Chainstate {
        &self.original_state
    }

    /// Returns if the new state matches the original state.
    ///
    /// This may be an expensive operation!
    pub fn is_empty(&self) -> bool {
        self.new_state == self.original_state
    }

    /// Finalizes the changes made to the state, exporting it as a write batch
    /// that can be applied to the previous state to produce it.
    pub fn finalize(self) -> WriteBatch {
        WriteBatch::new(self.new_state)
    }

    // Primitive manipulation functions.

    // Semantic manipulation functions.
    // TODO rework a lot of these to make them lower-level and focus more on
    // just keeping the core invariants consistent

    /// Sets the current slot.
    ///
    /// # Panics
    ///
    /// If this call does not cause the current slot to increase.
    pub fn set_slot(&mut self, slot: u64) {
        let state = self.state_mut();
        assert!(slot > state.cur_slot, "stateop: decreasing slot");
        state.cur_slot = slot;
    }

    /// Sets the last block commitment.
    pub fn set_prev_block(&mut self, block: L2BlockCommitment) {
        let state = self.state_mut();
        state.prev_block = block;
    }

    /// Sets the current epoch index.
    pub fn set_cur_epoch(&mut self, epoch: Epoch) {
        self.state_mut().cur_epoch = epoch;
    }

    /// Sets the previous epoch.
    pub fn set_prev_epoch(&mut self, epoch: EpochCommitment) {
        self.state_mut().prev_epoch = epoch;
    }

    /// Sets the previous epoch.
    pub fn set_finalized_epoch(&mut self, epoch: EpochCommitment) {
        self.state_mut().finalized_epoch = epoch;
    }

    pub fn set_epoch_finishing_flag(&mut self, flag: bool) {
        let state = self.state_mut();
        state.is_epoch_finishing = flag;
    }

    pub fn should_finish_epoch(&self) -> bool {
        self.state().is_epoch_finishing
    }

    /// Update HeaderVerificationState
    pub fn update_header_vs(&mut self, header: &Header) -> Result<(), L1VerificationError> {
        let state = self.state_mut();
        state.l1_state.header_vs.check_and_update(header)
    }

    /// Writes a deposit intent into an execution environment's input queue.
    pub fn insert_deposit_intent(&mut self, ee_id: u32, intent: DepositIntent) {
        assert_eq!(ee_id, 0, "stateop: only support execution env 0 right now");
        let state = self.state_mut();
        state.exec_env_state.pending_deposits.push_back(intent);
    }

    /// Remove a deposit intent from the pending deposits queue.
    ///
    /// This actually removes possibly multiple deposit intents.
    pub fn consume_deposit_intent(&mut self, idx: u64) {
        let deposits = self.state_mut().exec_env_state.pending_deposits_mut();

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

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct WriteBatchEntry {
    wb: WriteBatch,
    blockid: L2BlockId,
}

impl WriteBatchEntry {
    pub fn new(wb: WriteBatch, blockid: L2BlockId) -> Self {
        Self { wb, blockid }
    }

    pub fn to_parts(self) -> (WriteBatch, L2BlockId) {
        (self.wb, self.blockid)
    }

    pub fn toplevel_chainstate(&self) -> &Chainstate {
        self.wb.new_toplevel_state()
    }

    pub fn blockid(&self) -> &L2BlockId {
        &self.blockid
    }
}

impl From<WriteBatchEntry> for ChainstateEntry {
    fn from(value: WriteBatchEntry) -> Self {
        let (wb, blockid) = value.to_parts();
        ChainstateEntry::new(wb.into_toplevel(), blockid)
    }
}
