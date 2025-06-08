use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{crypto::Signature, error::DeserializeError};

/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config’s key list.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
pub struct AggregatedVote {
    indices: Vec<u8>,
    signature: Signature,
}

impl AggregatedVote {
    /// Create a new `AggregatedVote` with given voter indices and aggregated signature.
    pub fn new(indices: Vec<u8>, signature: Signature) -> Self {
        Self { indices, signature }
    }

    /// Borrow the aggregated signature.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Borrow the voter indices slice.
    pub fn voter_indices(&self) -> &[u8] {
        &self.indices
    }

    /// Consume and return the inner `(indices, signature)`.
    pub fn into_inner(self) -> (Vec<u8>, Signature) {
        (self.indices, self.signature)
    }

    /// Extract an `AggregatedVote` from a transaction input.
    ///
    /// FIXME: replace with actual deserialization logic.
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, DeserializeError> {
        // TODO: parse TxInput to obtain indices and aggregated signature
        Ok(Self::new(vec![0u8; 15], Signature::default()))
    }
}
