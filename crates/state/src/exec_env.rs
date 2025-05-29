//! Types relating to the state of an execution environment in the CL state.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{buf::Buf32, l1::payload::BlobSpec};

use crate::{bridge_ops, exec_update, forced_inclusion, state_queue::StateQueue};

#[derive(Debug, Clone, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ExecEnvState {
    /// The last processed exec update, which we've checked to be valid.  We may
    /// not have seen its DA blobs on the L1 yet.
    last_update_input: exec_update::UpdateInput,

    /// Current state root.
    cur_state: Buf32,

    /// DA blobs that have been indicated by a exec update, but haven't been
    /// seen on the corresponding DA layer yet.
    ///
    /// This must always be sorted.
    waiting_da_blobs: Vec<BlobSpec>,

    /// Deposits that have been queued by something but haven't been accepted in
    /// an update yet.  The sequencer should be processing these as soon as
    /// possible.
    // TODO make this not pub
    pub pending_deposits: StateQueue<bridge_ops::DepositIntent>,

    /// Forced inclusions that have been accepted by the CL but not processed by
    /// a CL payload yet.
    // TODO This is a stub, we don't support these yet and should assert it to
    // be empty.
    pending_forced_incls: StateQueue<forced_inclusion::ForcedInclusion>,
}

impl ExecEnvState {
    /// Constructs an env state from a starting input and the a state root,
    /// without producing any blobs, deposits, forced inclusions, etc.
    pub const fn from_base_input(base_input: exec_update::UpdateInput, state: Buf32) -> Self {
        Self {
            last_update_input: base_input,
            cur_state: state,
            waiting_da_blobs: Vec::new(),
            pending_deposits: StateQueue::new_empty(),
            pending_forced_incls: StateQueue::new_empty(),
        }
    }

    pub const fn update_idx(&self) -> u64 {
        self.last_update_input.update_idx()
    }

    pub const fn cur_state_root(&self) -> &Buf32 {
        &self.cur_state
    }

    pub const fn pending_deposits(&self) -> &StateQueue<bridge_ops::DepositIntent> {
        &self.pending_deposits
    }

    pub const fn pending_deposits_mut(&mut self) -> &mut StateQueue<bridge_ops::DepositIntent> {
        &mut self.pending_deposits
    }
}

impl<'a> Arbitrary<'a> for ExecEnvState {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let inp = exec_update::UpdateInput::arbitrary(u)?;
        let state = Buf32::arbitrary(u)?;
        Ok(Self::from_base_input(inp, state))
    }
}
