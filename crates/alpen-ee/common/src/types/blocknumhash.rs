use strata_acct_types::Hash;

/// A block identifier combining hash and height.
#[derive(Debug, Clone, Copy)]
pub struct BlockNumHash {
    /// Block hash
    pub hash: Hash,
    /// Block number
    pub height: u64,
}
