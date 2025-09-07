use bitvec::{slice::BitSlice, vec::BitVec};
use std::marker::PhantomData;

use crate::multisig::traits::CryptoScheme;

/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config's key list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedVote<S: CryptoScheme> {
    indices: BitVec,
    signature: S::Signature,
    /// Phantom data to carry the crypto scheme type.
    _phantom: PhantomData<S>,
}

impl<S: CryptoScheme> AggregatedVote<S> {
    /// Create a new `AggregatedVote` with given voter indices and aggregated signature.
    pub fn new(indices: BitVec, signature: S::Signature) -> Self {
        Self { 
            indices, 
            signature,
            _phantom: PhantomData,
        }
    }

    /// Borrow the aggregated signature.
    pub fn signature(&self) -> &S::Signature {
        &self.signature
    }

    /// Borrow the voter indices slice.
    pub fn voter_indices(&self) -> &BitSlice {
        &self.indices
    }

    /// Consume and return the inner `(indices, signature)`.
    pub fn into_inner(self) -> (BitVec, S::Signature) {
        (self.indices, self.signature)
    }
}

impl<S: CryptoScheme> Default for AggregatedVote<S>
where
    S::Signature: Default,
{
    fn default() -> Self {
        Self {
            indices: BitVec::default(),
            signature: S::Signature::default(),
            _phantom: PhantomData,
        }
    }
}
