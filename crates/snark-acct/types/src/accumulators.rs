//! Types relating to accumulators and making proofs against them.

use strata_acct_types::{Hash, MerkleProof};

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
    entry_hash: Hash,
    proof: MerkleProof,
}

impl MmrEntryProof {
    pub fn new(entry_hash: Hash, proof: MerkleProof) -> Self {
        Self { entry_hash, proof }
    }

    pub fn entry_hash(&self) -> &Hash {
        &self.entry_hash
    }

    pub fn proof(&self) -> &MerkleProof {
        &self.proof
    }

    pub fn entry_idx(&self) -> u64 {
        self.proof.index()
    }

    /// Converts the proof to a compact claim for the entry being proven.
    ///
    /// This doesn't verify the proof, this should only be called if we have
    /// reason to believe that the proof is valid.
    pub fn to_claim(&self) -> AccumulatorClaim {
        AccumulatorClaim::new(self.entry_idx(), *self.entry_hash())
    }
}
