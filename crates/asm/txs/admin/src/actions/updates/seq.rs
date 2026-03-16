use arbitrary::{Arbitrary, Unstructured};
use strata_primitives::buf::Buf32;

pub use crate::SequencerUpdate;
use crate::{actions::Sighash, constants::AdminTxType};

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

impl Sighash for SequencerUpdate {
    fn tx_type(&self) -> AdminTxType {
        AdminTxType::SequencerUpdate
    }

    fn sighash_payload(&self) -> Vec<u8> {
        self.pub_key.0.to_vec()
    }
}

impl<'a> Arbitrary<'a> for SequencerUpdate {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(Buf32::arbitrary(u)?))
    }
}
