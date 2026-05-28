//! Update message types.

use strata_acct_types::{MessageEntry, RawMerkleProof};
use tree_hash::TreeHash;

use crate::{
    AccumulatorClaim,
    ssz_generated::ssz::{outputs::UpdateOutputs, state::ProofState, update::*},
};

impl L1BlockRef {
    /// Creates an L1 block reference.
    pub fn new(block_hash: impl Into<[u8; 32]>, wtxids_root: impl Into<[u8; 32]>) -> Self {
        Self {
            block_hash: Into::<[u8; 32]>::into(block_hash).into(),
            wtxids_root: Into::<[u8; 32]>::into(wtxids_root).into(),
        }
    }

    /// Gets the referenced Bitcoin block hash.
    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash
            .as_ref()
            .try_into()
            .expect("snark-acct-types: FixedBytes<32> is always 32 bytes")
    }

    /// Gets the block witness transaction Merkle root.
    pub fn wtxids_root(&self) -> [u8; 32] {
        self.wtxids_root
            .as_ref()
            .try_into()
            .expect("snark-acct-types: FixedBytes<32> is always 32 bytes")
    }

    /// Computes the canonical OL L1 block refs MMR leaf hash.
    pub fn leaf_hash(&self) -> [u8; 32] {
        <L1BlockRef as TreeHash>::tree_hash_root(self).into_inner()
    }
}

/// Computes the canonical OL L1 block refs MMR leaf hash.
pub fn l1_block_ref_leaf_hash(block_hash: &[u8; 32], wtxids_root: &[u8; 32]) -> [u8; 32] {
    L1BlockRef::new(*block_hash, *wtxids_root).leaf_hash()
}

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
    pub fn new(l1_block_refs: Vec<AccumulatorClaim>) -> Self {
        Self {
            // FIXME does this panic?
            l1_block_refs: l1_block_refs
                .try_into()
                .expect("ledger refs must fit within SSZ max length"),
        }
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    /// Claims against the OL L1 block refs MMR.
    ///
    /// Each claim's `idx` is the L1 block height of the referenced block ref,
    /// and each `entry_hash` commits to `{blockhash, wtxids_root}`.
    pub fn l1_block_refs(&self) -> &[AccumulatorClaim] {
        self.l1_block_refs.as_ref()
    }
}

impl LedgerRefProofs {
    pub fn new(l1_block_ref_proofs: Vec<RawMerkleProof>) -> Self {
        Self {
            // FIXME does this panic?
            l1_block_ref_proofs: l1_block_ref_proofs
                .try_into()
                .expect("ledger ref proofs must fit within SSZ max length"),
        }
    }

    pub fn l1_block_ref_proofs(&self) -> &[RawMerkleProof] {
        self.l1_block_ref_proofs.as_ref()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_block_ref_leaf_hash_matches_pinned_root() {
        let block_hash = [1u8; 32];
        let wtxids_root = [2u8; 32];
        // Captured from `l1_block_ref_leaf_hash(&[1u8; 32], &[2u8; 32])` on a
        // known-good run; pinned here so any change to the `L1BlockRef` SSZ
        // TreeHash layout (field order, types, container shape) trips this test.
        let expected = [
            248, 24, 175, 211, 122, 109, 195, 188, 146, 251, 68, 115, 16, 17, 39, 112, 6, 219, 78,
            250, 110, 144, 35, 205, 116, 104, 192, 35, 53, 210, 42, 77,
        ];

        assert_eq!(l1_block_ref_leaf_hash(&block_hash, &wtxids_root), expected);
    }

    #[test]
    fn l1_block_ref_accessors_return_fixed_bytes() {
        let block_hash = [3u8; 32];
        let wtxids_root = [4u8; 32];
        let l1_block_ref = L1BlockRef::new(block_hash, wtxids_root);

        assert_eq!(l1_block_ref.block_hash(), block_hash);
        assert_eq!(l1_block_ref.wtxids_root(), wtxids_root);
    }
}
