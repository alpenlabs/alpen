//! Types relating to accumulators and making proofs against them.

use strata_identifiers::Hash;

use crate::AccumulatorClaim;

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
            .expect("acct-types: FixedBytes<32> is always 32 bytes")
    }
}
