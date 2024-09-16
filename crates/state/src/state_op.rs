//! Low-level operations we can make to write to chain state.  This currently
//! only can manipulate the manipulate the toplevel chain state, but we might
//! decide to expand the chain state in the future such that we can't keep it
//! entire in memory.

use alpen_express_primitives::buf::Buf32;
use borsh::{BorshDeserialize, BorshSerialize};
use tracing::*;

use crate::{
    bridge_ops, chain_state::ChainState, header::L2Header, id::L2BlockId, l1, l1::L1MaturationEntry,
};

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum StateOp {
    /// Replace the chain state with something completely different.
    Replace(Box<ChainState>),

    /// Sets the current slot.
    SetSlotAndTipBlock(u64, L2BlockId),

    /// Reverts L1 accepted height back to a previous height, rolling back any
    /// blocks that were there.
    RevertL1Height(u64),

    /// Accepts a new L1 block into the maturation queue.
    AcceptL1Block(l1::L1MaturationEntry),

    /// Matures the next L1 block, whose idx must match the one specified here
    /// as a sanity check.
    MatureL1Block(u64),

    /// Inserts a deposit intent into the pending deposits queue.
    EnqueueDepositIntent(bridge_ops::DepositIntent),

    /// Creates an operator
    CreateOperator(Buf32, Buf32),
}

/// Collection of writes we're making to the state.
#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct WriteBatch {
    ops: Vec<StateOp>,
}

impl WriteBatch {
    pub fn new(ops: Vec<StateOp>) -> Self {
        Self { ops }
    }

    pub fn new_replace(new_state: ChainState) -> Self {
        Self::new(vec![StateOp::Replace(Box::new(new_state))])
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }
}

// TODO reversiblity stuff?

/// On a given in-memory chainstate, applies a write batch.
///
/// This must succeed.  Pancis if it does not.
pub fn apply_write_batch_to_chainstate(
    mut chainstate: ChainState,
    batch: &WriteBatch,
) -> ChainState {
    for op in &batch.ops {
        apply_op_to_chainstate(op, &mut chainstate);
    }

    chainstate
}

fn apply_op_to_chainstate(op: &StateOp, state: &mut ChainState) {
    match op {
        StateOp::Replace(new_state) => *state = new_state.as_ref().clone(),

        StateOp::SetSlotAndTipBlock(slot, last_block) => {
            state.slot = *slot;
            state.last_block = *last_block;
        }

        StateOp::RevertL1Height(to_height) => {
            debug!(%to_height, "Obtained RevertL1Height Operation");
            let mqueue = &mut state.l1_state.maturation_queue;
            let back_idx = mqueue.back_idx().expect("stateop: maturation queue empty");

            // Do some bookkeeping to make sure it's safe to do this.
            if *to_height > back_idx {
                panic!("stateop: revert to above tip block");
            }

            let n_drop = back_idx - to_height;
            if n_drop > mqueue.len() as u64 {
                panic!("stateop: revert matured block");
            }

            // Now that it's safe to do the revert, we can just do it.
            for _ in 0..n_drop {
                // This expect should never trigger.
                mqueue.pop_back().expect("stateop: unable to revert more");
            }
        }

        StateOp::AcceptL1Block(entry) => {
            state.l1_state.maturation_queue.push_back(entry.clone());
        }

        StateOp::MatureL1Block(maturing_idx) => {
            let mqueue = &mut state.l1_state.maturation_queue;

            // Checks.
            assert!(mqueue.len() > 1); // make sure we'll still have blocks in the queue
            let front_idx = mqueue.front_idx().unwrap();
            assert_eq!(front_idx, *maturing_idx);

            // Actually take the block out so we can do something with it.
            let _matured_block = mqueue.pop_front();

            // TODO add it to the MMR so we can reference it in the future
            // TODO handle the DA txs and the deposit update txs, maybe in other ops
        }

        StateOp::EnqueueDepositIntent(intent) => {
            let deposits = state.exec_env_state.pending_deposits_mut();
            deposits.push_back(intent.clone());
        }

        StateOp::CreateOperator(spk, wpk) => {
            state.operator_table.insert(*spk, *wpk);
        }
    }
}

/// Cache that writes to state and remembers the series of operations made to it
/// so they can be persisted to disk without saving the chainstate.
///
/// If we ever have a large state that's persisted to disk, this will eventually
/// be made generic over a state provider that exposes access to that and then
/// the `WriteBatch` will include writes that can be made to that.
pub struct StateCache {
    state: ChainState,
    write_ops: Vec<StateOp>,
}

impl StateCache {
    pub fn new(state: ChainState) -> Self {
        Self {
            state,
            write_ops: Vec::new(),
        }
    }

    pub fn state(&self) -> &ChainState {
        &self.state
    }

    /// Finalizes the changes made to the state, exporting it and a write batch
    /// that can be applied to the previous state to produce it.
    pub fn finalize(self) -> (ChainState, WriteBatch) {
        (self.state, WriteBatch::new(self.write_ops))
    }

    /// Returns if the state cache is empty, meaning that no writes have been
    /// performed.
    pub fn is_empty(&self) -> bool {
        self.write_ops.is_empty()
    }

    /// Applies some operations to the state, including them in the write ops
    /// list.
    fn merge_ops(&mut self, ops: impl Iterator<Item = StateOp>) {
        for op in ops {
            apply_op_to_chainstate(&op, &mut self.state);
            self.write_ops.push(op);
        }
    }

    /// Like `merge_ops`, but only for a single op, for convenience.
    fn merge_op(&mut self, op: StateOp) {
        self.merge_ops([op].into_iter());
    }

    /// Sets the current slot in the state.
    pub fn set_cur_header(&mut self, header: &impl L2Header) {
        self.merge_op(StateOp::SetSlotAndTipBlock(
            header.blockidx(),
            header.get_blockid(),
        ));
    }

    /// Enqueues a deposit intent into the pending deposits queue.
    pub fn enqueue_deposit_intent(&mut self, intent: bridge_ops::DepositIntent) {
        self.merge_op(StateOp::EnqueueDepositIntent(intent));
    }

    /// Inserts a new operator with the specified pubkeys into the operator table.
    pub fn insert_operator(&mut self, signing_pk: Buf32, wallet_pk: Buf32) {
        self.merge_op(StateOp::CreateOperator(signing_pk, wallet_pk));
    }

    pub fn revert_l1_view_to(&mut self, height: u64) {
        self.merge_op(StateOp::RevertL1Height(height));
    }

    pub fn apply_l1_block_entry(&mut self, ent: L1MaturationEntry) {
        self.merge_op(StateOp::AcceptL1Block(ent));
    }

    // TODO add more manipulator functions
}
