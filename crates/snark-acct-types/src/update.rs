//! Update message types.

use crate::{
    accumulators::{AccumulatorClaim, MmrEntryProof},
    messages::{MessageEntry, MessageEntryProof},
    outputs::UpdateOutputs,
    state::ProofState,
};

/// Description of the operation of what we're updating.
#[derive(Clone, Debug)]
pub struct UpdateOperation {
    /// Sequence number to prevent replays, since we can't just rely on message
    /// index.
    seq_no: u64,

    /// The new state we're claiming.
    new_state: ProofState,

    /// Messages we processed in the inbox.
    processed_messages: Vec<MessageEntry>,

    /// Ledger references we're making.
    ledger_refs: LedgerRefs,

    /// Outputs we're emitting to update the ledger.
    outputs: UpdateOutputs,

    /// Arbitrary data we persist in DA.  This is formatted according to the
    /// needs of the snark account's application.
    extra_data: Vec<u8>,
}

impl UpdateOperation {
    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn new_state(&self) -> ProofState {
        self.new_state
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.processed_messages
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

/// Describes references to entries in accumulators available in the ledger.
///
/// These is generated from a [`LedgerRefProofs`].
#[derive(Clone, Debug)]
pub struct LedgerRefs {
    l1_header_refs: Vec<AccumulatorClaim>,
}

impl LedgerRefs {
    pub fn l1_header_refs(&self) -> &[AccumulatorClaim] {
        &self.l1_header_refs
    }
}

/// Container for references to ledger accumulators with proofs.
#[derive(Clone, Debug)]
pub struct LedgerRefProofs {
    l1_headers_proofs: Vec<MmrEntryProof>,
}

impl LedgerRefProofs {
    pub fn new(l1_headers_proofs: Vec<MmrEntryProof>) -> Self {
        Self { l1_headers_proofs }
    }

    pub fn l1_headers_proofs(&self) -> &[MmrEntryProof] {
        &self.l1_headers_proofs
    }

    /// Converts the proof structure to the entries claimed.  This should only
    /// happen after we've verified all of proofs against the accumulators that
    /// are being checked.
    pub fn to_ref_claims(&self) -> LedgerRefs {
        LedgerRefs {
            l1_header_refs: self
                .l1_headers_proofs
                .iter()
                .map(|e| e.to_claim())
                .collect::<Vec<_>>(),
        }
    }
}

/// Container for a snark account update with the various relevant proofs.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdate {
    /// The state change/requirements operation data itself.
    data: UpdateOperation,

    /// Proof for the update itself, attesting to relationships between the
    /// various fields.
    // TODO use predicate spec
    update_proof: Vec<u8>,

    /// MMR proofs for each of the inbox messages we processed.
    ///
    /// These may be updated by the sequencer.
    inbox_proofs: Vec<MessageEntryProof>,

    /// MMR proofs for each piece of ledger data we referenced.
    ///
    /// These may be updated by the sequencer.
    ledger_ref_proofs: LedgerRefProofs,
}
