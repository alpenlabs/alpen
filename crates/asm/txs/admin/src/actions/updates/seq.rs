use arbitrary::Arbitrary;
use strata_primitives::buf::Buf32;

/// An update to the public key of the sequencer.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
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
