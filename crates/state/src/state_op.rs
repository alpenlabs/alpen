//! Low-level operations we can make to write to chain state.
//!
//! This currently only can manipulate the toplevel chain state, but we might
//! decide to expand the chain state in the future such that we can't keep it
//! entire in memory.

use bitcoin::{block::Header, params::Params};
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{
    bridge::{BitcoinBlockHeight, OperatorIdx},
    buf::Buf32,
    epoch::EpochCommitment,
    l1::{
        BitcoinAmount, L1HeaderRecord, L1VerificationError, OutputRef, WithdrawalFulfillmentInfo,
    },
    l2::{L2BlockCommitment, L2BlockId},
};
use tracing::warn;

use crate::{
    bridge_ops::DepositIntent,
    bridge_state::{DepositEntry, DepositState, DispatchCommand, DispatchedState, FulfilledState},
    chain_state::{Chainstate, ChainstateEntry},
};

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
#[repr(u8)] // needed because of representational shit
pub enum StateOp {
    /// Does nothing, successfully.
    Noop,
}

/// Collection of writes we're making to the state.
#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct WriteBatch {
    /// Full "toplevel" state.
    new_toplevel_state: Chainstate,

    /// Ops applied to the "bulk state", which doesn't exist yet.
    ops: Vec<StateOp>,
}

impl WriteBatch {
    /// Creates a new instance from the toplevel state and a list of ops.
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(new_toplevel_state: Chainstate, ops: Vec<StateOp>) -> Self {
        Self {
            new_toplevel_state,
            ops,
        }
    }

    /// Creates a new instance from the new toplevel state and assumes no
    /// changes to the bulk state.
    pub fn new_replace(new_state: Chainstate) -> Self {
        Self::new(new_state, Vec::new())
    }

    pub const fn new_toplevel_state(&self) -> &Chainstate {
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

    /// Write operations we're making to the bulk state, if there are any.
    write_ops: Vec<StateOp>,
}

impl StateCache {
    pub fn new(state: Chainstate) -> Self {
        Self {
            original_state: state.clone(),
            new_state: state,
            write_ops: Vec::new(),
        }
    }

    // Basic accessors.

    pub const fn state(&self) -> &Chainstate {
        &self.new_state
    }

    pub const fn state_mut(&mut self) -> &mut Chainstate {
        &mut self.new_state
    }

    pub const fn original_state(&self) -> &Chainstate {
        &self.original_state
    }

    /// Returns if there's no write ops.
    ///
    /// Note that this does not guarantee that no changes have been made to the
    /// chainstate from wherever it was derived from before the instance was
    /// constructed.  This is a minimal safety measure.
    pub fn is_empty(&self) -> bool {
        self.write_ops.is_empty()
    }

    /// Finalizes the changes made to the state, exporting it as a write batch
    /// that can be applied to the previous state to produce it.
    pub fn finalize(self) -> WriteBatch {
        WriteBatch::new(self.new_state, self.write_ops)
    }

    // Primitive manipulation functions.

    /// Pushes a new state op onto the write ops list.
    ///
    /// This currently is meaningless since we don't have write ops that do anything anymore.
    #[deprecated(
        note = "there is no way to make use of this anymore, but we're leaving it in case we do have something to do with it"
    )]
    pub fn push_op(&mut self, op: StateOp) {
        self.write_ops.push(op);
    }

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
    pub const fn set_prev_block(&mut self, block: L2BlockCommitment) {
        let state = self.state_mut();
        state.prev_block = block;
    }

    /// Sets the current epoch index.
    pub const fn set_cur_epoch(&mut self, epoch: u64) {
        self.state_mut().cur_epoch = epoch;
    }

    /// Sets the previous epoch.
    pub const fn set_prev_epoch(&mut self, epoch: EpochCommitment) {
        self.state_mut().prev_epoch = epoch;
    }

    /// Sets the previous epoch.
    pub const fn set_finalized_epoch(&mut self, epoch: EpochCommitment) {
        self.state_mut().finalized_epoch = epoch;
    }

    /// Updates the safe L1 block.
    pub fn update_safe_block(&mut self, height: u64, record: L1HeaderRecord) {
        let state = self.state_mut();
        state.l1_state.safe_block_height = height;
        state.l1_state.safe_block_header = record;
    }

    pub const fn set_epoch_finishing_flag(&mut self, flag: bool) {
        let state = self.state_mut();
        state.is_epoch_finishing = flag;
    }

    pub const fn should_finish_epoch(&self) -> bool {
        self.state().is_epoch_finishing
    }

    /// Update HeaderVerificationState
    pub fn update_header_vs(
        &mut self,
        header: &Header,
        params: &Params,
    ) -> Result<(), L1VerificationError> {
        let state = self.state_mut();
        state
            .l1_state
            .header_vs
            .check_and_update_full(header, params)
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

    /// Inserts a new operator with the specified pubkeys into the operator table.
    pub fn insert_operator(&mut self, signing_pk: Buf32, wallet_pk: Buf32) {
        let state = self.state_mut();
        state.operator_table.insert(signing_pk, wallet_pk);
    }

    /// Inserts a new deposit with some settings.
    pub fn insert_deposit_entry(
        &mut self,
        idx: u32,
        tx_ref: OutputRef,
        amt: BitcoinAmount,
        operators: Vec<OperatorIdx>,
    ) -> bool {
        let dt = self.state_mut().deposits_table_mut();
        dt.try_create_deposit_at(idx, tx_ref, operators, amt)
    }

    /// Assigns a withdrawal command to a deposit, with an expiration.
    pub fn assign_withdrawal_command(
        &mut self,
        deposit_idx: u32,
        operator_idx: OperatorIdx,
        cmd: DispatchCommand,
        exec_height: BitcoinBlockHeight,
        withdrawal_txid: Buf32,
    ) {
        let deposit_ent = self
            .state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx");

        let state =
            DepositState::Dispatched(DispatchedState::new(cmd.clone(), operator_idx, exec_height));
        deposit_ent.set_state(state);
        deposit_ent.set_withdrawal_request_txid(Some(withdrawal_txid));
    }

    /// Updates the deposit assignee and expiration date.
    pub fn reset_deposit_assignee(
        &mut self,
        deposit_idx: u32,
        operator_idx: OperatorIdx,
        new_exec_height: BitcoinBlockHeight,
    ) {
        let deposit_ent = self
            .state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx");

        if let DepositState::Dispatched(dstate) = deposit_ent.deposit_state_mut() {
            dstate.set_assignee(operator_idx);
            dstate.set_exec_deadline(new_exec_height);
        } else {
            panic!("stateop: unexpected deposit state");
        };
    }

    /// Updates the deposit state to Fulfilled.
    pub fn mark_deposit_fulfilled(&mut self, winfo: &WithdrawalFulfillmentInfo) {
        let deposit_ent = self.deposit_entry_mut_expect(winfo.deposit_idx);

        let oidx = winfo.operator_idx;
        let is_valid = deposit_ent.deposit_state().is_dispatched_to(oidx);

        assert!(is_valid, "stateop: incorrect deposit state dispatch");

        deposit_ent.set_state(DepositState::Fulfilled(FulfilledState::new(
            winfo.operator_idx,
            winfo.amt,
            winfo.txid,
        )));
    }

    // Updates the deposit state as Reimbursed.
    pub fn mark_deposit_reimbursed(&mut self, deposit_idx: u32) {
        let deposit_ent = self.deposit_entry_mut_expect(deposit_idx);

        if !matches!(deposit_ent.deposit_state(), DepositState::Fulfilled(_)) {
            // TODO: handle this better after TN1 bridge is integrated
            warn!("stateop: deposit spent at unexpected state");
        }

        deposit_ent.set_state(DepositState::Reimbursed);
    }

    fn deposit_entry_mut_expect(&mut self, deposit_idx: u32) -> &mut DepositEntry {
        self.state_mut()
            .deposits_table_mut()
            .get_deposit_mut(deposit_idx)
            .expect("stateop: missing deposit idx")
    }

    // TODO add more manipulator functions
}

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct WriteBatchEntry {
    wb: WriteBatch,
    blockid: L2BlockId,
}

impl WriteBatchEntry {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(wb: WriteBatch, blockid: L2BlockId) -> Self {
        Self { wb, blockid }
    }

    pub fn to_parts(self) -> (WriteBatch, L2BlockId) {
        (self.wb, self.blockid)
    }

    pub const fn toplevel_chainstate(&self) -> &Chainstate {
        self.wb.new_toplevel_state()
    }

    pub const fn blockid(&self) -> &L2BlockId {
        &self.blockid
    }
}

impl From<WriteBatchEntry> for ChainstateEntry {
    fn from(value: WriteBatchEntry) -> Self {
        let (wb, blockid) = value.to_parts();
        ChainstateEntry::new(wb.into_toplevel(), blockid)
    }
}
