use strata_identifiers::OLBlockCommitment;

#[derive(Debug)]
pub struct OlChainStatus {
    pub latest: OLBlockCommitment,
    pub confirmed: OLBlockCommitment,
    pub finalized: OLBlockCommitment,
}

impl OlChainStatus {
    pub fn latest(&self) -> &OLBlockCommitment {
        &self.latest
    }
    pub fn confirmed(&self) -> &OLBlockCommitment {
        &self.confirmed
    }
    pub fn finalized(&self) -> &OLBlockCommitment {
        &self.finalized
    }
}
