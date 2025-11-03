//! Update message types.

use crate::{
    accumulators::{AccumulatorClaim, MmrEntryProof},
    messages::{MessageEntry, MessageEntryProof},
    outputs::UpdateOutputs,
    state::ProofState,
};

/// Description of the operation of what we're updating.
#[derive(Clone, Debug)]
pub struct UpdateOperationData {
    /// Subset of data that is sufficient for unconditional (trusted) updates.
    unconditional: UpdateOperationUnconditionalData,

    /// Ledger references we're making.
    ledger_refs: LedgerRefs,

    /// Outputs we're emitting to update the ledger.
    outputs: UpdateOutputs,
}

impl UpdateOperationData {
    pub fn new(
        seq_no: u64,
        new_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            unconditional: UpdateOperationUnconditionalData {
                seq_no,
                new_state,
                processed_messages,
                extra_data,
            },
            ledger_refs,
            outputs,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.unconditional.seq_no
    }

    pub fn new_state(&self) -> ProofState {
        self.unconditional.new_state
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.unconditional.processed_messages
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.unconditional.extra_data
    }

    pub fn as_unconditional(&self) -> &UpdateOperationUnconditionalData {
        &self.unconditional
    }
}

/// Subset of UpdateOperationData fields needed for unconditional update.
/// This represents all the data of an update operation that is available to a DA-only synced OL
/// fullnode.
#[derive(Clone, Debug)]
pub struct UpdateOperationUnconditionalData {
    /// Sequence number to prevent replays, since we can't just rely on message
    /// index.
    seq_no: u64,

    /// The new state we're claiming.
    new_state: ProofState,

    /// Messages we processed in the inbox.
    processed_messages: Vec<MessageEntry>,

    /// Arbitrary data we persist in DA.  This is formatted according to the
    /// needs of the snark account's application.
    extra_data: Vec<u8>,
}

impl UpdateOperationUnconditionalData {
    pub fn new(
        seq_no: u64,
        new_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            seq_no,
            new_state,
            processed_messages,
            extra_data,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn new_state(&self) -> ProofState {
        self.new_state
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.processed_messages
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

impl From<UpdateOperationData> for UpdateOperationUnconditionalData {
    fn from(value: UpdateOperationData) -> Self {
        value.unconditional
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
    pub fn new(l1_header_refs: Vec<AccumulatorClaim>) -> Self {
        Self { l1_header_refs }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

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

/// Container for a snark account update but with only the update proof itself,
/// ignoring the accumulator proofs that anyone can theoretically generate.
///
/// This is enough to verify that an update is safe to potentially apply to some
/// current state, but not that its claimed dependencies on the ledger are
/// actually correct.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdate {
    /// The state change/requirements operation data itself.
    operation: UpdateOperationData,

    /// Proof for the update itself, attesting to relationships between the
    /// various fields.
    // TODO use predicate spec
    update_proof: Vec<u8>,
}

impl SnarkAccountUpdate {
    pub fn new(operation: UpdateOperationData, update_proof: Vec<u8>) -> Self {
        Self {
            operation,
            update_proof,
        }
    }

    pub fn operation(&self) -> &UpdateOperationData {
        &self.operation
    }

    pub fn update_proof(&self) -> &[u8] {
        &self.update_proof
    }

    /// Converts the base snark account update and converts it into the full
    /// version by providing accumulator proofs.
    ///
    /// The proofs MUST correspond to the accumulator requirements.  This DOES
    /// NOT validate that they are correct, this must be checked ahead of time.
    pub fn into_full(self, proofs: UpdateAccumulatorProofs) -> SnarkAccountUpdateContainer {
        SnarkAccountUpdateContainer {
            base_update: self,
            accumulator_proofs: proofs,
        }
    }
}

/// The proofs for the inputs and ledger references that we accessing in the
/// ledger.
///
/// Note that this container does not specify *which block* these proofs are for
/// as that must be supplied from some additional context.
#[derive(Clone, Debug)]
pub struct UpdateAccumulatorProofs {
    /// MMR proofs for each of the inbox messages we processed.
    ///
    /// These may be updated by the sequencer.
    inbox_proofs: Vec<MessageEntryProof>,

    /// MMR proofs for each piece of ledger data we referenced.
    ///
    /// These may be updated by the sequencer.
    ledger_ref_proofs: LedgerRefProofs,
}

impl UpdateAccumulatorProofs {
    fn new(inbox_proofs: Vec<MessageEntryProof>, ledger_ref_proofs: LedgerRefProofs) -> Self {
        Self {
            inbox_proofs,
            ledger_ref_proofs,
        }
    }

    pub fn inbox_proofs(&self) -> &[MessageEntryProof] {
        &self.inbox_proofs
    }

    pub fn ledger_ref_proofs(&self) -> &LedgerRefProofs {
        &self.ledger_ref_proofs
    }
}

/// Container for a snark account update with the contextual relevant proofs.
///
/// This is what is contained in the OL block.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdateContainer {
    /// The base update data with proof which can be checked independently.
    base_update: SnarkAccountUpdate,

    /// Proofs for the "context" around the ledger that we're operating on,
    /// which may need to be updated according to the recent state if it was
    /// provided for an older one.
    accumulator_proofs: UpdateAccumulatorProofs,
}

impl SnarkAccountUpdateContainer {
    pub fn new(
        base_update: SnarkAccountUpdate,
        accumulator_proofs: UpdateAccumulatorProofs,
    ) -> Self {
        Self {
            base_update,
            accumulator_proofs,
        }
    }

    pub fn base_update(&self) -> &SnarkAccountUpdate {
        &self.base_update
    }

    pub fn accumulator_proofs(&self) -> &UpdateAccumulatorProofs {
        &self.accumulator_proofs
    }

    /// Gets the inner operation data that we do stuff with.
    pub fn operation(&self) -> &UpdateOperationData {
        self.base_update().operation()
    }
}
