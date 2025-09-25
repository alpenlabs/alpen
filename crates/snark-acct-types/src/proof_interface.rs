//! Proof interface types.

use crate::{
    messages::MessageEntry, outputs::UpdateOutputs, state::ProofState, update::LedgerRefs,
};

/// Public params that we provide as the claim the proof must prove the relate
/// to each other correctly.
#[derive(Clone, Debug)]
pub struct UpdateProofPubParams {
    /// Current state we're extending.
    cur_state: ProofState,

    /// New state we're trying to prove.
    new_state: ProofState,

    /// Messages from the inbox we're accepting.
    message_inputs: Vec<MessageEntry>,

    /// Checked claims for other accumulators/state on the ledger.
    ledger_refs: LedgerRefs,

    /// Outputs from the account which will be applied, modifying the state of
    /// the ledger.
    outputs: UpdateOutputs,

    /// The extra data field from the update operation which will be persisted
    /// in DA.
    extra_data: Vec<u8>,
}

impl UpdateProofPubParams {
    pub fn cur_state(&self) -> ProofState {
        self.cur_state
    }

    pub fn new_state(&self) -> ProofState {
        self.new_state
    }

    pub fn message_inputs(&self) -> &[MessageEntry] {
        &self.message_inputs
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}
