use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_identifiers::Buf32;

/// An update to the public key of the sequencer.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct SequencerUpdate {
    pub_key: Buf32,
}

impl SequencerUpdate {
    /// Create a new `SequencerUpdate` from the given public key.
    pub fn new(pub_key: Buf32) -> Self {
        Self { pub_key }
    }

    /// Borrow the new sequencer public key.
    pub fn pub_key(&self) -> &Buf32 {
        &self.pub_key
    }

    /// Consume and return the inner public key.
    pub fn into_inner(self) -> Buf32 {
        self.pub_key
    }
}
