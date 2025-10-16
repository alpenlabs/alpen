//! Snark account state types.

use ssz_derive::{Decode, Encode};
use strata_acct_types::{AccountTypeId, AccountTypeState, Hash, Mmr64, impl_opaque_thin_wrapper};
use tree_hash::TreeHash;
use tree_hash_derive::TreeHash as TreeHashDerive;

/// State root type.
type Root = Hash;

/// Raw sequence number type.
type RawSeqno = u64;

/// Account sequence number type.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct Seqno(RawSeqno);

// Manual TreeHash implementation for transparent wrapper
impl TreeHash for Seqno {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u64 as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u64 as TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <u64 as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u64 as TreeHash>::tree_hash_root(&self.0)
    }
}

impl_opaque_thin_wrapper!(Seqno => RawSeqno);

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
pub struct SnarkAccountState {
    /// Vk used to verify updates.
    update_vk: Vec<u8>, // TODO use predicate fmt

    /// The proof state that gets updated by updates.
    proof_state: ProofState,

    /// Sequence number for updates.
    seq_no: Seqno,

    /// Inbox message MMR.
    inbox_mmr: Mmr64,
}

impl SnarkAccountState {
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

impl AccountTypeState for SnarkAccountState {
    const ID: AccountTypeId = AccountTypeId::Snark;
}

/// Snark account's proof state, updated on a proof.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode, TreeHashDerive,
)]
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

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_seqno_ssz_roundtrip() {
        let seqno = Seqno::new(42);
        let encoded = seqno.as_ssz_bytes();
        let decoded = Seqno::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(seqno, decoded);
    }

    #[test]
    fn test_seqno_tree_hash() {
        let seqno = Seqno::new(1000);
        let hash = seqno.tree_hash_root();
        // Should produce same hash as underlying u64
        assert_eq!(hash, <u64 as TreeHash>::tree_hash_root(&1000u64));
    }

    #[test]
    fn test_seqno_incr() {
        let seqno = Seqno::new(5);
        let next = seqno.incr();
        assert_eq!(*next.inner(), 6);

        // Test SSZ after increment
        let encoded = next.as_ssz_bytes();
        let decoded = Seqno::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(next, decoded);
    }

    #[test]
    fn test_proof_state_ssz_roundtrip() {
        let state = ProofState::new([42u8; 32], 100);

        let encoded = state.as_ssz_bytes();
        let decoded = ProofState::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(state, decoded);
        assert_eq!(state.inner_state(), decoded.inner_state());
        assert_eq!(state.next_inbox_msg_idx(), decoded.next_inbox_msg_idx());
    }

    #[test]
    fn test_proof_state_tree_hash() {
        let state1 = ProofState::new([1u8; 32], 50);
        let state2 = ProofState::new([1u8; 32], 50);

        // Same state should produce same hash
        assert_eq!(state1.tree_hash_root(), state2.tree_hash_root());

        // Different state should produce different hash
        let state3 = ProofState::new([2u8; 32], 50);
        assert_ne!(state1.tree_hash_root(), state3.tree_hash_root());
    }

    #[test]
    fn test_proof_state_zero_values() {
        let state = ProofState::new([0u8; 32], 0);

        let encoded = state.as_ssz_bytes();
        let decoded = ProofState::from_ssz_bytes(&encoded).unwrap();

        assert_eq!(state, decoded);
        assert_eq!(decoded.next_inbox_msg_idx(), 0);
    }
}
