//! Snark account state types.

use std::mem;

use ssz_derive::{Decode, Encode};
use strata_acct_types::{AccountTypeId, AccountTypeState, Hash, Mmr64, impl_opaque_thin_wrapper};
use strata_identifiers::impl_ssz_transparent_wrapper;

use crate::ssz_generated::ssz::state::*;

/// Raw sequence number type.
type RawSeqno = u64;

/// Size of RawSeqno (u64) in bytes
const RAW_SEQNO_LEN: usize = mem::size_of::<RawSeqno>();

/// Account sequence number type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[ssz(struct_behaviour = "transparent")]
pub struct Seqno(RawSeqno);

impl_opaque_thin_wrapper!(Seqno => RawSeqno);

impl_ssz_transparent_wrapper!(Seqno, RawSeqno, RAW_SEQNO_LEN);

impl Seqno {
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Gets the incremented seqno.
    pub fn incr(self) -> Self {
        // do we really have to panic here?
        if *self.inner() == RawSeqno::MAX {
            panic!("snarkacct: reached max seqno");
        }

        Seqno::new(self.inner() + 1)
    }
}

impl ProofState {
    /// Creates a new proof state.
    pub fn new(inner_state: Hash, next_inbox_msg_idx: u64) -> Self {
        Self {
            inner_state: Into::<[u8; 32]>::into(inner_state).into(),
            next_inbox_msg_idx,
        }
    }

    /// Gets the inner state commitment.
    pub fn inner_state(&self) -> Hash {
        self.inner_state
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> is always 32 bytes")
    }

    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }
}

impl SnarkAccountState {
    pub fn update_vk(&self) -> &[u8] {
        &self.update_vk
    }

    pub fn proof_state(&self) -> ProofState {
        self.proof_state.clone()
    }

    pub fn seq_no(&self) -> Seqno {
        Seqno::new(self.seq_no)
    }

    pub fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }
}

impl AccountTypeState for SnarkAccountState {
    const ID: AccountTypeId = AccountTypeId::Snark;
}
