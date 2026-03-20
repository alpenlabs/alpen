use strata_acct_types::Hash;
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

/// Status of the OL chain including tip, confirmed and finalized epochs.
#[derive(Debug, Clone, Copy)]
pub struct OLChainStatus {
    /// Tip block commitment.
    pub tip: OLBlockCommitment,

    /// Confirmed epoch commitment.
    pub confirmed: EpochCommitment,

    /// Finalized epoch commitment.
    pub finalized: EpochCommitment,
}

impl OLChainStatus {
    /// Returns the tip block commitment.
    pub fn tip(&self) -> &OLBlockCommitment {
        &self.tip
    }

    /// Returns the confirmed epoch commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized epoch commitment.
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
