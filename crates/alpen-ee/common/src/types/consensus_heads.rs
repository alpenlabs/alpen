use strata_acct_types::Hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusHeads {
    pub confirmed: Hash,
    pub finalized: Hash,
}

impl ConsensusHeads {
    pub fn confirmed(&self) -> &Hash {
        &self.confirmed
    }

    pub fn finalized(&self) -> &Hash {
        &self.finalized
    }
}
