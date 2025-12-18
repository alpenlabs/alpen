use strata_acct_types::{AcctResult, Hash, Mmr64};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_ledger_types::*;
use strata_merkle::CompactMmr64B32;
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{MessageEntry, Seqno};
use tree_hash::TreeHash;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSnarkAccountState {
    verification_key: PredicateKey,
    seqno: Seqno,
    proof_state: ProofState,
    inbox_mmr: Mmr64,
}

impl NativeSnarkAccountState {
    /// Creates an account instance with specific values.
    pub(crate) fn new(
        verification_key: PredicateKey,
        seqno: Seqno,
        proof_state: ProofState,
        inbox_mmr: Mmr64,
    ) -> Self {
        Self {
            verification_key,
            seqno,
            proof_state,
            inbox_mmr,
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
        &self.verification_key
    }

    fn seqno(&self) -> Seqno {
        self.seqno
    }

    fn inner_state_root(&self) -> Hash {
        self.proof_state.inner_state_root
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }
}

impl ISnarkAccountStateMut for NativeSnarkAccountState {
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
        // TODO maybe document this a little better?
        let hash = <MessageEntry as TreeHash>::tree_hash_root(&entry);
        self.inbox_mmr
            .add_leaf(hash.into_inner())
            .expect("ol/state: mmr add_leaf");
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Codec)]
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

impl Codec for NativeSnarkAccountState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode PredicateKey using SSZ
        let vk_bytes = ssz::Encode::as_ssz_bytes(&self.verification_key);
        let wrapped_vk = CodecSsz::new(vk_bytes);
        wrapped_vk.encode(enc)?;

        self.seqno.encode(enc)?;
        self.proof_state.encode(enc)?;

        // Convert Mmr64 to CompactMmr64B32 and encode it using CodecSsz wrapper
        let compact_mmr = self.inbox_mmr.to_compact();
        let wrapped_mmr = CodecSsz::new(compact_mmr);
        wrapped_mmr.encode(enc)?;

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode PredicateKey from SSZ bytes
        let wrapped_vk: CodecSsz<Vec<u8>> = CodecSsz::decode(dec)?;
        let vk_bytes = wrapped_vk.inner();
        let verification_key = ssz::Decode::from_ssz_bytes(vk_bytes)
            .map_err(|_| CodecError::InvalidVariant("PredicateKey"))?;

        let seqno = Seqno::decode(dec)?;
        let proof_state = ProofState::decode(dec)?;

        // Decode the CompactMmr64B32 using CodecSsz wrapper and convert back to Mmr64
        let wrapped_mmr: CodecSsz<CompactMmr64B32> = CodecSsz::decode(dec)?;
        let compact_mmr = wrapped_mmr.inner();
        let inbox_mmr = Mmr64::from_compact(compact_mmr);

        Ok(Self {
            verification_key,
            seqno,
            proof_state,
            inbox_mmr,
        })
    }
}
