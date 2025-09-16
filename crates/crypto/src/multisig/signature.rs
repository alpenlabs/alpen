use std::marker::PhantomData;

use bitvec::{slice::BitSlice, vec::BitVec};

use crate::multisig::traits::CryptoScheme;

/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config's key list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedSignature<S: CryptoScheme> {
    indices: BitVec<u8>,
    signature: S::Signature,
    /// Phantom data to carry the crypto scheme type.
    _phantom: PhantomData<S>,
}

impl<S: CryptoScheme> AggregatedSignature<S> {
    /// Create a new `AggregatedSignature` with given signer indices and aggregated signature.
    pub fn new(indices: BitVec<u8>, signature: S::Signature) -> Self {
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

    /// Borrow the signer indices slice.
    pub fn signer_indices(&self) -> &BitSlice<u8> {
        &self.indices
    }

    /// Consume and return the inner `(indices, signature)`.
    pub fn into_inner(self) -> (BitVec<u8>, S::Signature) {
        (self.indices, self.signature)
    }
}

impl<S: CryptoScheme> Default for AggregatedSignature<S>
where
    S::Signature: Default,
{
    fn default() -> Self {
        Self {
            indices: BitVec::<u8>::default(),
            signature: S::Signature::default(),
            _phantom: PhantomData,
        }
    }
}
