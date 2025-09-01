use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::MultisigAction;
use strata_crypto::multisig::{
    aggregate_pubkeys, config::MultisigConfig, errors::VoteValidationError, msg::MultisigPayload,
    verify_sig, vote::AggregatedVote,
};
use strata_primitives::{hash::compute_borsh_hash, roles::Role};

/// Manages multisignature operations for a given role and key set, with replay protection via a
/// nonce.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    role: Role,
    /// The public keys of all grant-holders authorized to sign.
    config: MultisigConfig,
    /// Sequence number for the multisig configuration. It increases on each valid action.
    /// This is used to prevent replay attacks
    seqno: u64,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: MultisigConfig) -> Self {
        Self {
            role,
            config,
            seqno: 0,
        }
    }

    /// The role authorized to perform multisig operations.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Borrow the current multisig configuration.
    pub fn config(&self) -> &MultisigConfig {
        &self.config
    }

    /// Mutably borrow the multisig configuration.
    pub fn config_mut(&mut self) -> &mut MultisigConfig {
        &mut self.config
    }

    /// Validate that `vote` approves `action` under the current config and nonce.
    ///
    /// Steps:
    /// 1. Map voter indices to pubkeys (error if out of bounds).
    /// 2. Aggregate pubkeys and compute payload hash.
    /// 3. Verify aggregated signature.
    pub fn validate_action(
        &self,
        action: &MultisigAction,
        vote: &AggregatedVote,
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
        let msg_hash = compute_borsh_hash(action);
        let payload = MultisigPayload::new(msg_hash, self.seqno);

        // 4. Verify the aggregated signature against the aggregated pubkey
        if !verify_sig(&aggregated_key, &payload, vote.signature()) {
            return Err(VoteValidationError::InvalidVoteSignature);
        }

        Ok(())
    }

    /// Increments the nonce.
    pub fn increment_seqno(&mut self) {
        self.seqno += 1;
    }
}
