//! Update message types.

use crate::{
    message::{MessageEntry, MessageEntryProof},
    outputs::UpdateOutputs,
    state::ProofState,
};

/// Description of the operation of what we're updating.
#[derive(Clone, Debug)]
pub struct UpdateOperation {
    /// The new state we're claiming.
    new_state: ProofState,

    /// Sequence number to prevent replays, since we can't just rely on message
    /// index.
    seq_no: u64,

    /// Messages we processed in the inbox.
    processed_messages: Vec<MessageEntry>,

    /// Ledger references we're making.
    ledger_refs: LedgerRefs,

    /// Outputs we're emitting to update the ledger.
    outputs: UpdateOutputs,

    /// Arbitrary data we persist in DA.
    extra_data: Vec<u8>,
}

impl UpdateOperation {
    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.processed_messages
    }
}

#[derive(Clone, Debug)]
pub struct InboxMes {
    proof: (), // TODO,
}

#[derive(Clone, Debug)]
pub struct LedgerRefs {
    // TODO
}

#[derive(Clone, Debug)]
pub struct LedgerRefProofs {
    // TODO
}

#[derive(Clone, Debug)]
pub struct SnarkAccountUpdate {
    /// The state change/requirements operation data itself.
    data: UpdateOperation,

    /// Proof for the update itself.
    // TODO use predicate spec
    base_proof: Vec<u8>,

    /// MMR proofs for each of the inbox messages we processed.
    ///
    /// These may be updated by the sequencer.
    inbox_proofs: Vec<MessageEntryProof>,

    /// MMR proofs for each piece of ledger data we referenced.
    ///
    /// These may be updated by the sequencer.
    ledger_ref_proofs: LedgerRefProofs,
}
