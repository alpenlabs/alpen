//! Proof interface types.

use crate::{
    message::MessageEntry,
    state::ProofState,
    update::{LedgerRefs, UpdateOutputs},
};

/// Public params that we provide as the claim the proof must prove.
#[derive(Clone, Debug)]
pub struct UpdateProofPubParams {
    cur_state: ProofState,
    new_state: ProofState,
    message_inputs: Vec<MessageEntry>,
    ledger_refs: LedgerRefs,
    outputs: UpdateOutputs,
}
