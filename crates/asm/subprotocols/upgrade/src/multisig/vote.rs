use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::UpgradeAction,
    crypto::{PubKey, Signature, aggregate_pubkeys, verify_sig},
    error::VoteValidationError,
};

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

    /// Validates this aggregated vote against the provided signer public keys for the given
    /// `action_id`.
    ///
    /// This method performs three steps:
    /// 1. Collect the individual public keys corresponding to `voter_indices` from `signers`.
    /// 2. Aggregate those public keys into a single `PubKey` using `aggregate_pubkeys`.
    /// 3. Verify the aggregated signature against the aggregated public key
    pub fn validate_action(
        &self,
        signers: &[PubKey],
        action: &UpgradeAction,
    ) -> Result<(), VoteValidationError> {
        // 1. Collect each public key by index; error if out of bounds.
        let signer_keys: Vec<PubKey> = self
            .voter_indices
            .iter()
            .map(|&i| {
                signers
                    .get(i as usize)
                    .cloned()
                    .ok_or(VoteValidationError::AggregationError)
            })
            .collect::<Result<_, _>>()?;

        // 2. Aggregate those public keys into one.
        let aggregated_key = aggregate_pubkeys(&signer_keys)?;

        // 3. Compute the action ID from the UpgradeAction
        let msg_hash = action.compute_id().into();

        // 3. Verify the aggregated signature against the aggregated pubkey
        if !verify_sig(&aggregated_key, &msg_hash, &self.agg_signature) {
            return Err(VoteValidationError::InvalidVoteSignature);
        }

        Ok(())
    }
}
