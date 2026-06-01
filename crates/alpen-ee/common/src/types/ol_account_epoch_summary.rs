use strata_acct_types::MessageEntry;
use strata_identifiers::{EpochCommitment, Hash};

/// One snark-account update as fetched by the alpen-client OL tracker.
///
/// `new_state_root` is `None` for intermediate updates fetched from a
/// checkpoint-sync source.
#[derive(Clone, Debug)]
pub struct SnarkAccountUpdateInfo {
    seq_no: u64,
    extra_data: Vec<u8>,
    messages: Vec<MessageEntry>,
    new_state_root: Option<Hash>,
}

impl SnarkAccountUpdateInfo {
    pub fn new(
        seq_no: u64,
        extra_data: Vec<u8>,
        messages: Vec<MessageEntry>,
        new_state_root: Option<Hash>,
    ) -> Self {
        Self {
            seq_no,
            extra_data,
            messages,
            new_state_root,
        }
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    pub fn messages(&self) -> &[MessageEntry] {
        &self.messages
    }

    pub fn new_state_root(&self) -> Option<Hash> {
        self.new_state_root
    }
}

#[derive(Debug)]
pub struct OLEpochSummary {
    epoch: EpochCommitment,
    prev: EpochCommitment,
    final_state_root: Hash,
    updates: Vec<SnarkAccountUpdateInfo>,
}

impl OLEpochSummary {
    pub fn new(
        epoch: EpochCommitment,
        prev: EpochCommitment,
        final_state_root: Hash,
        updates: Vec<SnarkAccountUpdateInfo>,
    ) -> Self {
        Self {
            epoch,
            prev,
            final_state_root,
            updates,
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        EpochCommitment,
        EpochCommitment,
        Hash,
        Vec<SnarkAccountUpdateInfo>,
    ) {
        (self.epoch, self.prev, self.final_state_root, self.updates)
    }

    pub fn epoch(&self) -> &EpochCommitment {
        &self.epoch
    }

    pub fn prev_epoch(&self) -> &EpochCommitment {
        &self.prev
    }

    pub fn final_state_root(&self) -> Hash {
        self.final_state_root
    }

    pub fn updates(&self) -> &[SnarkAccountUpdateInfo] {
        &self.updates
    }
}
