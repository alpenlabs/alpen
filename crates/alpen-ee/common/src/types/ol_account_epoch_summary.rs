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
    new_next_msg_idx: u64,
}

impl SnarkAccountUpdateInfo {
    pub fn new(
        seq_no: u64,
        extra_data: Vec<u8>,
        messages: Vec<MessageEntry>,
        new_state_root: Option<Hash>,
        new_next_msg_idx: u64,
    ) -> Self {
        Self {
            seq_no,
            extra_data,
            messages,
            new_state_root,
            new_next_msg_idx,
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

    /// Inbox cursor after this update is applied.
    pub fn new_next_msg_idx(&self) -> u64 {
        self.new_next_msg_idx
    }

    /// Iterates this update's messages paired with their absolute inbox indexes.
    ///
    /// Indexes start at `new_next_msg_idx - messages.len()` and increment by one
    /// per message. Useful for logging which inbox entries an update consumed.
    pub fn iter_messages_with_idxs(&self) -> impl Iterator<Item = (u64, &MessageEntry)> {
        let start = self.new_next_msg_idx - self.messages.len() as u64;
        self.messages
            .iter()
            .enumerate()
            .map(move |(i, m)| (start + i as u64, m))
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
