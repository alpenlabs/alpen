use super::bridge::OperatorIdx;
use crate::prelude::Buf32;

// Re-export OperatorPubkeys from btc-types
pub use strata_btc_types::OperatorPubkeys;

/// Some type that can provide operator keys.
pub trait OperatorKeyProvider {
    /// Returns the operator's signing pubkey, if it exists in the table.
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32>;
}

/// Stub key provider that can be used for testing.
#[derive(Debug)]
pub struct StubOpKeyProv {
    expected_idx: OperatorIdx,
    pk: Buf32,
}

impl StubOpKeyProv {
    pub fn new(expected_idx: OperatorIdx, pk: Buf32) -> Self {
        Self { expected_idx, pk }
    }
}

impl OperatorKeyProvider for StubOpKeyProv {
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32> {
        if idx == self.expected_idx {
            Some(self.pk)
        } else {
            None
        }
    }
}
