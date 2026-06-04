//! Update history types extracted from L1.

use strata_acct_types::{Hash, MessageEntry};

/// Snark account update extracted from L1, used to reconstruct inner state
/// outside the proof.
///
/// `new_state_root` is `None` when the caller cannot supply a per-update root
/// (checkpoint sync only recovers the terminal epoch state). The apply path
/// asserts post-state when present and skips when absent.
#[derive(Clone, Debug)]
pub struct UpdateManifest {
    new_state_root: Option<Hash>,
    extra_data: Vec<u8>,
    messages: Vec<MessageEntry>,
}

impl UpdateManifest {
    pub fn new(
        new_state_root: Option<Hash>,
        extra_data: Vec<u8>,
        messages: Vec<MessageEntry>,
    ) -> Self {
        Self {
            new_state_root,
            extra_data,
            messages,
        }
    }

    pub fn new_state_root(&self) -> Option<Hash> {
        self.new_state_root
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    pub fn messages(&self) -> &[MessageEntry] {
        &self.messages
    }
}
