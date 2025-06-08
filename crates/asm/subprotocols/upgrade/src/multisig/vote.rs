use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{crypto::Signature, error::DeserializeError};

/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config’s key list.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq, Eq, Default)]
pub struct AggregatedVote {
    voter_indices: Vec<u8>,
    agg_signature: Signature,
}

impl AggregatedVote {
    pub fn new(voter_indices: Vec<u8>, agg_signature: Signature) -> Self {
        Self {
            voter_indices,
            agg_signature,
        }
    }

    pub fn signature(&self) -> &Signature {
        &self.agg_signature
    }

    pub fn voter_indices(&self) -> &[u8] {
        &self.voter_indices
    }
}

impl AggregatedVote {
    // FIXME:
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, DeserializeError> {
        let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());
        Ok(vote)
    }
}
