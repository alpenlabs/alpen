use strata_acct_types::{AcctResult, Hash, Mmr64};
use strata_ledger_types::ISnarkAccountState;
use strata_snark_acct_types::{MessageEntry, Seqno};

#[derive(Clone, Debug)]
pub struct NativeSnarkAccountState {
    seqno: Seqno,
    proof_state: ProofState,
    inbox_mmr: Mmr64,
}

impl NativeSnarkAccountState {
    /// Creates an account instance with specific values.
    pub(crate) fn new(seqno: Seqno, proof_state: ProofState, inbox_mmr: Mmr64) -> Self {
        Self {
            seqno,
            proof_state,
            inbox_mmr,
        }
    }

    /// Creates a new fresh instance with a particular initial state, but other
    /// bookkeeping set to 0.
    pub fn new_fresh(initial_state_root: Hash) -> Self {
        let ps = ProofState::new(initial_state_root, 0);
        Self::new(Seqno::zero(), ps, Mmr64::new(64))
    }
}

impl ISnarkAccountState for NativeSnarkAccountState {
    fn seqno(&self) -> Seqno {
        self.seqno
    }

    fn inner_state_root(&self) -> Hash {
        self.proof_state.inner_state_root
    }

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

    fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }

    fn insert_inbox_message(&mut self, _entry: MessageEntry) -> AcctResult<()> {
        // TODO ssz hash entry, append to mmr
        todo!()
    }
}

#[derive(Clone, Debug)]
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

    pub fn inner_state_root(&self) -> [u8; 32] {
        self.inner_state_root
    }

    pub fn next_msg_read_idx(&self) -> u64 {
        self.next_msg_read_idx
    }
}
