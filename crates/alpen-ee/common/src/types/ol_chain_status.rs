use strata_acct_types::Hash;
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// Status of the OL chain including latest, confirmed, and finalized blocks.
#[derive(Debug, Clone, Copy)]
pub struct OLChainStatus {
    /// Latest block commitment.
    pub latest: OLBlockCommitment,
    /// Confirmed block commitment.
    pub confirmed: EpochCommitment,
    /// Finalized block commitment.
    pub finalized: EpochCommitment,
}

impl OLChainStatus {
    /// Returns the latest block commitment.
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }

    /// Returns the confirmed block commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized block commitment.
    pub fn finalized(&self) -> &EpochCommitment {
        &self.finalized
    }
}

/// Finalized OL block and its corresponding EE block hash.
#[derive(Debug, Clone, Copy)]
pub struct OLFinalizedStatus {
    /// finalized ol block.
    pub ol_block: OLBlockCommitment,
    /// blockhash of last ee block whose update was posted upto this ol block.
    pub last_ee_block: Hash,
}
