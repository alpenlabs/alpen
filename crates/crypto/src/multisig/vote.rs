use bitvec::{slice::BitSlice, vec::BitVec};

use crate::multisig::Signature;

/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config’s key list.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AggregatedVote {
    indices: BitVec,
    signature: Signature,
}

impl AggregatedVote {
    /// Create a new `AggregatedVote` with given voter indices and aggregated signature.
    pub fn new(indices: BitVec, signature: Signature) -> Self {
        Self { indices, signature }
    }

    /// Borrow the aggregated signature.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Borrow the voter indices slice.
    pub fn voter_indices(&self) -> &BitSlice {
        &self.indices
    }

    /// Consume and return the inner `(indices, signature)`.
    pub fn into_inner(self) -> (BitVec, Signature) {
        (self.indices, self.signature)
    }
}
