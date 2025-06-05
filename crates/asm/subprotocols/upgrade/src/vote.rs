use borsh::{BorshDeserialize, BorshSerialize};

use crate::types::Signature;

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
}
