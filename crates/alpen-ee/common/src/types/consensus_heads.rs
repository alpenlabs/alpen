use strata_acct_types::Hash;

/// Consensus block hashes representing confirmed and finalized states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusHeads {
    /// Confirmed block hash.
    pub confirmed: Hash,
    /// Finalized block hash.
    pub finalized: Hash,
}

impl ConsensusHeads {
    /// Returns the confirmed block hash.
    pub fn confirmed(&self) -> &Hash {
        &self.confirmed
    }

    /// Returns the finalized block hash.
    pub fn finalized(&self) -> &Hash {
        &self.finalized
    }
}
