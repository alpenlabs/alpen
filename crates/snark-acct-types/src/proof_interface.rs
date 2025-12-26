//! Proof interface types.

// Import the SSZ-generated UpdateProofPubParams
pub use crate::ssz_generated::ssz::update::UpdateProofPubParams;
use crate::{LedgerRefs, MessageEntry, ProofState, UpdateOutputs};

impl UpdateProofPubParams {
    /// Creates a new UpdateProofPubParams
    pub fn new(
        cur_state: ProofState,
        new_state: ProofState,
        message_inputs: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            cur_state,
            new_state,
            message_inputs: message_inputs.into(),
            ledger_refs,
            outputs,
            extra_data: extra_data.into(),
        }
    }

    pub fn cur_state(&self) -> ProofState {
        self.cur_state.clone()
    }

    pub fn new_state(&self) -> ProofState {
        self.new_state.clone()
    }

    pub fn message_inputs(&self) -> &[MessageEntry] {
        self.message_inputs.as_ref()
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    pub fn extra_data(&self) -> &[u8] {
        self.extra_data.as_ref()
    }
}
