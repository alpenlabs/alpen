//! Types relating to accumulators and making proofs against them.

type Hash = [u8; 32];

// TODO make this use the MMR crate
type MmrProof = Vec<u8>;

/// Claim that an entry exists in a linear accumulator at some index.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AccumulatorClaim {
    idx: u64, // maybe 32
    entry_hash: Hash,
}

impl AccumulatorClaim {
    pub fn new(idx: u64, entry_hash: Hash) -> Self {
        Self { idx, entry_hash }
    }

    pub fn idx(&self) -> u64 {
        self.idx
    }

    pub fn entry_hash(&self) -> &Hash {
        &self.entry_hash
    }
}

/// Claim with proof for an entry in an MMR.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MmrEntryProof {
    claim: AccumulatorClaim,
    proof: MmrProof,
}

impl MmrEntryProof {
    pub fn new(claim: AccumulatorClaim, proof: Vec<u8>) -> Self {
        Self { claim, proof }
    }

    pub fn claim(&self) -> &AccumulatorClaim {
        &self.claim
    }

    pub fn idx(&self) -> u64 {
        self.claim.idx()
    }

    pub fn entry_hash(&self) -> &Hash {
        self.claim.entry_hash()
    }
}
