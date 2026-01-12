use strata_acct_types::Hash;

/// A block identifier combining hash and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockNumHash {
    /// Block hash
    hash: Hash,
    /// Block number
    height: u64,
}

impl BlockNumHash {
    /// Create new [`BlockNumHash`].
    pub fn new(hash: Hash, height: u64) -> Self {
        Self { hash, height }
    }

    /// Block hash.
    pub fn hash(&self) -> Hash {
        self.hash
    }

    /// Block number.
    pub fn blocknum(&self) -> u64 {
        self.height
    }
}
