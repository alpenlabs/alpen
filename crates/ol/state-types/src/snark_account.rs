use ssz_types::VariableList;
use strata_acct_types::{AcctResult, Hash, Mmr64, StrataHasher, tree_hash::TreeHash};
use strata_ledger_types::*;
use strata_merkle::{CompactMmr64, Mmr, Mmr64B32};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::ssz_generated::ssz::state::{OLSnarkAccountState, ProofState};

impl OLSnarkAccountState {
    /// Creates an account instance with specific values.
    pub(crate) fn new(
        verifying_key: PredicateKey,
        seqno: Seqno,
        proof_state: ProofState,
        inbox_mmr: Mmr64,
        update_vk: Vec<u8>,
    ) -> Self {
        let update_vk = VariableList::new(update_vk).expect("ol/state: update vk exceeds limit");
        Self {
            verifying_key,
            seqno,
            proof_state,
            inbox_mmr,
            update_vk,
        }
    }

    /// Creates a new fresh instance with a particular initial state, but other
    /// bookkeeping set to 0.
    pub fn new_fresh(
        verification_key: PredicateKey,
        initial_state_root: Hash,
        update_vk: Vec<u8>,
    ) -> Self {
        let ps = ProofState::new(initial_state_root, 0);
        let generic_mmr = CompactMmr64::<[u8; 32]>::new(64);
        let mmr64 = Mmr64::from_generic(&generic_mmr);
        Self::new(verification_key, Seqno::zero(), ps, mmr64, update_vk)
    }
}

impl ISnarkAccountState for OLSnarkAccountState {
    fn verifying_key(&self) -> &PredicateKey {
        &self.verifying_key
    }

    fn seqno(&self) -> Seqno {
        self.seqno
    }

    fn inner_state_root(&self) -> Hash {
        self.proof_state.inner_state_root()
    }

    fn next_inbox_msg_idx(&self) -> u64 {
        self.proof_state.next_msg_read_idx
    }

    fn update_vk(&self) -> &[u8] {
        self.update_vk.as_ref()
    }

    fn inbox_mmr(&self) -> &Mmr64B32 {
        &self.inbox_mmr
    }
}

impl ISnarkAccountStateMut for OLSnarkAccountState {
    fn set_proof_state_directly(&mut self, state: Hash, next_read_idx: u64, seqno: Seqno) {
        self.proof_state = ProofState::new(state, next_read_idx);
        self.seqno = seqno;
    }

    fn update_inner_state(
        &mut self,
        state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        _extra_data: &[u8],
    ) -> AcctResult<()> {
        // Set the proof state but ignore extra data in this context.
        self.set_proof_state_directly(state, next_read_idx, seqno);
        Ok(())
    }

    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()> {
        let hash = <MessageEntry as TreeHash>::tree_hash_root(&entry);
        Mmr::<StrataHasher>::add_leaf(&mut self.inbox_mmr, hash.into_inner())
            .expect("ol/state: mmr add_leaf");
        Ok(())
    }
}

impl ProofState {
    pub fn new(inner_state_root: Hash, next_msg_read_idx: u64) -> Self {
        // Convert Hash (Buf32) to [u8; 32] then to FixedBytes<32>
        let hash_bytes: [u8; 32] = inner_state_root.into();
        Self {
            inner_state_root: hash_bytes.into(),
            next_msg_read_idx,
        }
    }

    pub fn inner_state_root(&self) -> Hash {
        // Convert FixedBytes<32> to [u8; 32] then to Hash (Buf32)
        let bytes: &[u8] = self.inner_state_root.as_ref();
        let arr: [u8; 32] = bytes.try_into().expect("FixedBytes<32> is always 32 bytes");
        Hash::from(arr)
    }

    pub fn next_msg_read_idx(&self) -> u64 {
        self.next_msg_read_idx
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_identifiers::Buf32;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::proof_state_strategy;

    mod proof_state {
        use super::*;

        ssz_proptest!(ProofState, proof_state_strategy());

        #[test]
        fn test_proof_state_basic() {
            let state = ProofState::new(Buf32::zero(), 42);
            assert_eq!(state.next_msg_read_idx(), 42);

            let encoded = state.as_ssz_bytes();
            let decoded = ProofState::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(state, decoded);
        }
    }
}
