use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::hash::compute_borsh_hash;

use crate::{
    crypto::{aggregate_pubkeys, verify_sig},
    error::VoteValidationError,
    multisig::{
        config::MultisigConfig,
        msg::{MultisigOp, MultisigPayload},
        vote::AggregatedVote,
    },
    roles::Role,
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    pub role: Role,
    /// The public keys of all grant-holders authorized to sign.
    pub config: MultisigConfig,
    /// Nonce for the multisig configuration.
    /// This is used to prevent replay attacks
    pub nonce: u64,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: MultisigConfig) -> Self {
        Self {
            role,
            config,
            nonce: 0,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn config(&self) -> &MultisigConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut MultisigConfig {
        &mut self.config
    }

    pub fn validate_op(
        &self,
        vote: &AggregatedVote,
        op: MultisigOp,
    ) -> Result<(), VoteValidationError> {
        // 1. Collect each public key by index; error if out of bounds.
        let signer_keys: Vec<_> = vote
            .voter_indices()
            .iter()
            .map(|&i| {
                self.config
                    .keys()
                    .get(i as usize)
                    .cloned()
                    .ok_or(VoteValidationError::AggregationError)
            })
            .collect::<Result<_, _>>()?;

        // 2. Aggregate those public keys into one.
        let aggregated_key = aggregate_pubkeys(&signer_keys)?;

        // 3. Compute the msg from the UpgradeAction
        let msg = MultisigPayload::new(op, self.nonce);
        let msg_hash = compute_borsh_hash(&msg);

        // 4. Verify the aggregated signature against the aggregated pubkey
        if !verify_sig(&aggregated_key, &msg_hash, vote.signature()) {
            return Err(VoteValidationError::InvalidVoteSignature);
        }

        Ok(())
    }
}
