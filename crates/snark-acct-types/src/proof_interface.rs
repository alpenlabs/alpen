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

    /// Messsages from the inbox we're accepting.
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
