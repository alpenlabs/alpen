//! Update history types extracted from L1.

use crate::{MessageEntry, ProofState};

/// Description of a snark account update extracted from L1.
///
/// This is used to compute and sanity check snark account inner states in
/// absence of orchestration layer blocks and coinputs.  Correctness is implied
/// by the orchestration layer permitting the state transition in the first
/// place, since that requires a snark proof.
#[derive(Clone, Debug)]
pub struct UpdateManifest {
    new_state: ProofState,
    extra_data: Vec<u8>,
    messages: Vec<MessageEntry>,
}

impl UpdateManifest {
    pub fn new(new_state: ProofState, extra_data: Vec<u8>, messages: Vec<MessageEntry>) -> Self {
        Self {
            new_state,
            extra_data,
            messages,
        }
    }

    pub fn new_state(&self) -> &ProofState {
        &self.new_state
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    pub fn messages(&self) -> &[MessageEntry] {
        &self.messages
    }
}
