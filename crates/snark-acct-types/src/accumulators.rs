//! Types relating to accumulators and making proofs against them.

use strata_acct_types::{Hash, MerkleProof};

use crate::ssz_generated::ssz::accumulators::{AccumulatorClaim, MmrEntryProof};

impl AccumulatorClaim {
    /// Creates a new accumulator claim.
    pub fn new(idx: u64, entry_hash: impl Into<[u8; 32]>) -> Self {
        Self {
            idx,
            entry_hash: Into::<[u8; 32]>::into(entry_hash).into(),
        }
    }

    /// Gets the index.
    pub fn idx(&self) -> u64 {
        self.idx
    }

    /// Gets the entry hash.
    pub fn entry_hash(&self) -> Hash {
        self.entry_hash
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> is always 32 bytes")
    }
}

impl MmrEntryProof {
    /// Creates a new MMR entry proof.
    pub fn new(entry_hash: impl Into<[u8; 32]>, proof: MerkleProof) -> Self {
        Self {
            entry_hash: Into::<[u8; 32]>::into(entry_hash).into(),
            proof,
        }
    }

    /// Gets the entry hash.
    pub fn entry_hash(&self) -> Hash {
        self.entry_hash
            .as_ref()
            .try_into()
            .expect("FixedBytes<32> is always 32 bytes")
    }

    /// Gets the proof.
    pub fn proof(&self) -> &MerkleProof {
        &self.proof
    }

    /// Gets the entry index from the proof.
    pub fn entry_idx(&self) -> u64 {
        self.proof.index()
    }

    /// Converts the proof to a compact claim for the entry being proven.
    ///
    /// This doesn't verify the proof, this should only be called if we have
    /// reason to believe that the proof is valid.
    pub fn to_claim(&self) -> AccumulatorClaim {
        AccumulatorClaim::new(self.entry_idx(), self.entry_hash())
    }
}
