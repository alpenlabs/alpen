//! Update message types.

use crate::ssz_generated::ssz::{
    accumulators::*, messages::*, outputs::UpdateOutputs, state::ProofState, update::*,
};

impl UpdateStateData {
    pub fn new(proof_state: ProofState, extra_data: Vec<u8>) -> Self {
        Self {
            proof_state,
            extra_data: extra_data.into(),
        }
    }

    pub fn proof_state(&self) -> ProofState {
        self.proof_state.clone()
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

impl UpdateInputData {
    pub fn new(seq_no: u64, messages: Vec<MessageEntry>, update_state: UpdateStateData) -> Self {
        Self {
            seq_no,
            messages: messages.into(),
            update_state,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn new_state(&self) -> ProofState {
        self.update_state.proof_state()
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.messages
    }

    pub fn extra_data(&self) -> &[u8] {
        self.update_state.extra_data()
    }
}

impl UpdateOperationData {
    pub fn new(
        seq_no: u64,
        proof_state: ProofState,
        messages: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            input: UpdateInputData {
                seq_no,
                messages: messages.into(),
                update_state: UpdateStateData {
                    proof_state,
                    extra_data: extra_data.into(),
                },
            },
            ledger_refs,
            outputs,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.input.seq_no()
    }

    pub fn new_state(&self) -> ProofState {
        self.input.new_state()
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        self.input.processed_messages()
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    pub fn extra_data(&self) -> &[u8] {
        self.input.extra_data()
    }

    pub fn as_input_data(&self) -> &UpdateInputData {
        &self.input
    }
}

impl From<UpdateOperationData> for UpdateInputData {
    fn from(value: UpdateOperationData) -> Self {
        value.input
    }
}

impl LedgerRefs {
    pub fn new(l1_header_refs: Vec<AccumulatorClaim>) -> Self {
        Self {
            l1_header_refs: l1_header_refs.into(),
        }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn l1_header_refs(&self) -> &[AccumulatorClaim] {
        &self.l1_header_refs
    }
}

impl LedgerRefProofs {
    pub fn new(l1_headers_proofs: Vec<MmrEntryProof>) -> Self {
        Self {
            l1_headers_proofs: l1_headers_proofs.into(),
        }
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
                .collect::<Vec<_>>()
                .into(),
        }
    }
}

impl SnarkAccountUpdate {
    pub fn new(operation: UpdateOperationData, update_proof: Vec<u8>) -> Self {
        Self {
            operation,
            update_proof: update_proof.into(),
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

impl UpdateAccumulatorProofs {
    pub fn new(inbox_proofs: Vec<MessageEntryProof>, ledger_ref_proofs: LedgerRefProofs) -> Self {
        Self {
            inbox_proofs: inbox_proofs.into(),
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
