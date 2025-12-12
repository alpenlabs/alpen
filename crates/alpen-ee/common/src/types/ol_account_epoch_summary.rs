use strata_identifiers::EpochCommitment;
use strata_snark_acct_types::UpdateInputData;

#[derive(Debug)]
pub struct OLEpochSummary {
    epoch: EpochCommitment,
    prev: EpochCommitment,
    updates: Vec<UpdateInputData>,
}

impl OLEpochSummary {
    pub fn new(
        epoch: EpochCommitment,
        prev: EpochCommitment,
        updates: Vec<UpdateInputData>,
    ) -> Self {
        Self {
            epoch,
            prev,
            updates,
        }
    }

    pub fn into_parts(self) -> (EpochCommitment, EpochCommitment, Vec<UpdateInputData>) {
        (self.epoch, self.prev, self.updates)
    }

    pub fn epoch(&self) -> &EpochCommitment {
        &self.epoch
    }

    pub fn prev_epoch(&self) -> &EpochCommitment {
        &self.prev
    }

    pub fn updates(&self) -> &[UpdateInputData] {
        &self.updates
    }
}
