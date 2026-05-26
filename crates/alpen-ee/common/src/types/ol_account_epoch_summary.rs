use strata_acct_types::MessageEntry;
use strata_identifiers::{EpochCommitment, Hash};

/// One snark-account update as fetched by the alpen-client OL tracker.
///
/// `final_state_root` is `None` when the source is a checkpoint-sync node:
/// only the terminal epoch state root is recoverable from DA, not per-update.
#[derive(Clone, Debug)]
pub struct EpochUpdateOp {
    pub seq_no: u64,
    pub extra_data: Vec<u8>,
    pub messages: Vec<MessageEntry>,
    pub final_state_root: Option<Hash>,
}

#[derive(Debug)]
pub struct OLEpochSummary {
    epoch: EpochCommitment,
    prev: EpochCommitment,
    updates: Vec<EpochUpdateOp>,
}

impl OLEpochSummary {
    pub fn new(epoch: EpochCommitment, prev: EpochCommitment, updates: Vec<EpochUpdateOp>) -> Self {
        Self {
            epoch,
            prev,
            updates,
        }
    }

    pub fn into_parts(self) -> (EpochCommitment, EpochCommitment, Vec<EpochUpdateOp>) {
        (self.epoch, self.prev, self.updates)
    }

    pub fn epoch(&self) -> &EpochCommitment {
        &self.epoch
    }

    pub fn prev_epoch(&self) -> &EpochCommitment {
        &self.prev
    }

    pub fn updates(&self) -> &[EpochUpdateOp] {
        &self.updates
    }
}
