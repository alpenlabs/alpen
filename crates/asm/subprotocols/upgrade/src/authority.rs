use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::UpgradeAction,
    crypto::{aggregate_pubkeys, verify_sig},
    error::VoteValidationError,
    multisig_config::MultisigConfig,
    roles::Role,
    vote::AggregatedVote,
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    pub role: Role,
    /// The public keys of all grant-holders authorized to sign.
    pub config: MultisigConfig,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: MultisigConfig) -> Self {
        Self { role, config }
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

    pub fn validate_action(
        &self,
        vote: &AggregatedVote,
        action: &UpgradeAction,
    ) -> Result<(), VoteValidationError> {
        // sanity check: ensure the action matches the authority's role
        assert_eq!(action.role(), self.role());

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
        let msg_hash = action.compute_id().into();

        // 4. Verify the aggregated signature against the aggregated pubkey
        if !verify_sig(&aggregated_key, &msg_hash, vote.signature()) {
            return Err(VoteValidationError::InvalidVoteSignature);
        }

        Ok(())
    }
}

