//! Snark account state types.

use strata_acct_types::{AcctTypeId, AcctTypeState, Hash, Mmr64, impl_thin_wrapper};

/// State root type.
type Root = Hash;

/// Raw sequence number type.
type RawSeqno = u64;

/// Account sequence number type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Seqno(RawSeqno);

impl_thin_wrapper!(Seqno => RawSeqno);

impl Seqno {
    /// Gets the incremented seqno.
    pub fn incr(self) -> Seqno {
        // do we really have to panic here?
        if *self.inner() == RawSeqno::MAX {
            panic!("snarkacct: reached max seqno");
        }

        Seqno::new(self.inner() + 1)
    }
}

/// Snark account state.  This is contained immediately within the basic
/// account state entry.
// TODO SSZ
#[derive(Clone, Debug)]
pub struct SnarkAcctState {
    /// Vk used to verify updates.
    update_vk: Vec<u8>, // TODO use predicate fmt

    /// The proof state that gets updated by updates.
    proof_state: ProofState,

    /// Sequence number for updates.
    seq_no: Seqno,

    /// Inbox message MMR.
    inbox_mmr: Mmr64,
}

impl SnarkAcctState {
    pub fn update_vk(&self) -> &[u8] {
        &self.update_vk
    }

    pub fn proof_state(&self) -> ProofState {
        self.proof_state
    }

    pub fn seq_no(&self) -> Seqno {
        self.seq_no
    }

    pub fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }
}

impl AcctTypeState for SnarkAcctState {
    const ID: AcctTypeId = AcctTypeId::Snark;
}

/// Snark account's proof state, updated on a proof.
// TODO SSZ
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProofState {
    /// Commitment to the internal state of the account, as defined by the
    /// proofs.
    inner_state: Root,

    /// The index of the next message a proof is expected to process.
    next_inbox_msg_idx: u64,
}

impl ProofState {
    pub fn new(inner_state: Root, next_inbox_msg_idx: u64) -> Self {
        Self {
            inner_state,
            next_inbox_msg_idx,
        }
    }

    pub fn inner_state(&self) -> [u8; 32] {
        self.inner_state
    }

    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}
