//! Update message types.

use strata_acct_types::{MessageEntry, RawMerkleProof};

use crate::{
    AccumulatorClaim,
    ssz_generated::ssz::{outputs::UpdateOutputs, state::ProofState, update::*},
};

impl UpdateStateData {
    pub fn new(proof_state: ProofState, extra_data: Vec<u8>) -> Self {
        Self {
            proof_state,
            // FIXME does this panic?
            extra_data: extra_data
                .try_into()
                .expect("snark account extra data must fit within SSZ max length"),
        }
    }

    pub fn proof_state(&self) -> ProofState {
        self.proof_state.clone()
    }

    pub fn extra_data(&self) -> &[u8] {
        self.extra_data.as_ref()
    }

    /// Replaces the update state's extra data in-place.
    pub fn set_extra_data(&mut self, extra_data: Vec<u8>) {
        self.extra_data = extra_data
            .try_into()
            .expect("snark account extra data must fit within SSZ max length");
    }
}

impl UpdateInputData {
    pub fn new(seq_no: u64, messages: Vec<MessageEntry>, update_state: UpdateStateData) -> Self {
        Self {
            seq_no,
            // FIXME does this panic?
            messages: messages
                .try_into()
                .expect("snark account messages must fit within SSZ max length"),
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
        self.messages.as_ref()
    }

    pub fn extra_data(&self) -> &[u8] {
        self.update_state.extra_data()
    }

    /// Replaces the update's extra data in-place.
    pub fn set_extra_data(&mut self, extra_data: Vec<u8>) {
        self.update_state.set_extra_data(extra_data);
    }
}

impl UpdateOperationData {
    pub fn new(
        seq_no: u64,
        proof_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        // TODO rework?
        Self {
            input: UpdateInputData::new(
                seq_no,
                processed_messages,
                UpdateStateData::new(proof_state, extra_data),
            ),
            ledger_refs,
            outputs,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.input.seq_no()
    }

    pub fn new_proof_state(&self) -> ProofState {
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
            // FIXME does this panic?
            l1_header_refs: l1_header_refs
                .try_into()
                .expect("ledger refs must fit within SSZ max length"),
        }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn l1_header_refs(&self) -> &[AccumulatorClaim] {
        self.l1_header_refs.as_ref()
    }
}

impl LedgerRefProofs {
    pub fn new(l1_headers_proofs: Vec<RawMerkleProof>) -> Self {
        Self {
            // FIXME does this panic?
            l1_headers_proofs: l1_headers_proofs
                .try_into()
                .expect("ledger ref proofs must fit within SSZ max length"),
        }
    }

    pub fn l1_headers_proofs(&self) -> &[RawMerkleProof] {
        self.l1_headers_proofs.as_ref()
    }
}

impl SnarkAccountUpdate {
    pub fn new(operation: UpdateOperationData, update_proof: Vec<u8>) -> Self {
        Self {
            operation,
            // FIXME does this panic?
            update_proof: update_proof
                .try_into()
                .expect("update proof bytes must fit within SSZ max length"),
        }
    }

    pub fn operation(&self) -> &UpdateOperationData {
        &self.operation
    }

    pub fn update_proof(&self) -> &[u8] {
        self.update_proof.as_ref()
    }
}

impl UpdateAccumulatorProofs {
    pub fn new(inbox_proofs: Vec<RawMerkleProof>, ledger_ref_proofs: LedgerRefProofs) -> Self {
        Self {
            // FIXME does this panic?
            inbox_proofs: inbox_proofs
                .try_into()
                .expect("inbox proofs must fit within SSZ max length"),
            ledger_ref_proofs,
        }
    }

    pub fn inbox_proofs(&self) -> &[RawMerkleProof] {
        self.inbox_proofs.as_ref()
    }

    pub fn ledger_ref_proofs(&self) -> &LedgerRefProofs {
        &self.ledger_ref_proofs
    }
}
