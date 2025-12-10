use strata_identifiers::OLBlockCommitment;

/// Status of the OL chain including latest, confirmed, and finalized blocks.
#[derive(Debug)]
pub struct OLChainStatus {
    /// Latest block commitment.
    pub latest: OLBlockCommitment,
    /// Confirmed block commitment.
    pub confirmed: OLBlockCommitment,
    /// Finalized block commitment.
    pub finalized: OLBlockCommitment,
}

impl OLChainStatus {
    /// Returns the latest block commitment.
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }

    /// Returns the confirmed block commitment.
    pub fn confirmed(&self) -> &OLBlockCommitment {
        &self.confirmed
    }

    /// Returns the finalized block commitment.
    pub fn finalized(&self) -> &OLBlockCommitment {
        &self.finalized
    }
}
