use ssz_derive::{Decode, Encode};
use strata_acct_types::{AcctResult, Hash, Mmr64};
use strata_codec::Codec;
use strata_codec_utils::CodecSsz;
use strata_ledger_types::*;
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{MessageEntry, Seqno};
use tree_hash::TreeHash;

#[derive(Clone, Debug, Eq, PartialEq, Codec, Decode, Encode)]
pub struct NativeSnarkAccountState {
    verification_key: CodecSsz<PredicateKey>,
    seqno: CodecSsz<Seqno>,
    proof_state: CodecSsz<ProofState>,
    inbox_mmr: CodecSsz<Mmr64>,
}

impl NativeSnarkAccountState {
    /// Creates an account instance with specific values.
    pub(crate) fn new(
        vk: PredicateKey,
        seqno: Seqno,
        proof_state: ProofState,
        inbox_mmr: Mmr64,
    ) -> Self {
        Self {
            verification_key: CodecSsz::new(vk),
            seqno: CodecSsz::new(seqno),
            proof_state: CodecSsz::new(proof_state),
            inbox_mmr: CodecSsz::new(inbox_mmr),
        }
    }

    /// Creates a new fresh instance with a particular initial state, but other
    /// bookkeeping set to 0.
    pub fn new_fresh(verification_key: PredicateKey, initial_state_root: Hash) -> Self {
        let ps = ProofState::new(initial_state_root, 0);
        Self::new(verification_key, Seqno::zero(), ps, Mmr64::new(64))
    }
}

impl ISnarkAccountState for NativeSnarkAccountState {
    fn verification_key(&self) -> &PredicateKey {
        self.verification_key.inner()
    }

    fn seqno(&self) -> Seqno {
        *self.seqno.inner()
    }

    fn inner_state_root(&self) -> Hash {
        self.proof_state.inner().inner_state_root
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        self.inbox_mmr.inner()
    }
}

impl ISnarkAccountStateMut for NativeSnarkAccountState {
    fn set_proof_state_directly(&mut self, state: Hash, next_read_idx: u64, seqno: Seqno) {
        let ps = ProofState::new(state, next_read_idx);
        self.proof_state = CodecSsz::new(ps);
        self.seqno = CodecSsz::new(seqno);
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
        // TODO maybe document this a little better?
        let hash = <MessageEntry as TreeHash>::tree_hash_root(&entry);
        self.inbox_mmr
            .inner_mut()
            .add_leaf(hash.into_inner())
            .expect("ol/state: mmr add_leaf");
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Codec, Decode, Encode)]
pub struct ProofState {
    inner_state_root: Hash,
    next_msg_read_idx: u64,
}

impl ProofState {
    pub fn new(inner_state_root: Hash, next_msg_read_idx: u64) -> Self {
        Self {
            inner_state_root,
            next_msg_read_idx,
        }
    }

    pub fn inner_state_root(&self) -> Hash {
        self.inner_state_root
    }

    pub fn next_msg_read_idx(&self) -> u64 {
        self.next_msg_read_idx
    }
}
