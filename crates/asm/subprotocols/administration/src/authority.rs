use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_administration_txs::actions::MultisigAction;
use strata_crypto::multisig::{AggregatedVote, MultisigConfig, MultisigError, verify_multisig};
use strata_primitives::roles::Role;

/// Manages multisignature operations for a given role and key set, with replay protection via a
/// nonce.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    role: Role,
    /// The public keys of all grant-holders authorized to sign.
    config: MultisigConfig,
    /// Sequence number for the multisig configuration. It increases on each valid action.
    /// This is used to prevent replay attacks.
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
    /// Uses the generic multisig verification function to orchestrate the workflow.
    pub fn validate_action(
        &self,
        action: &MultisigAction,
        vote: &AggregatedVote,
    ) -> Result<(), MultisigError> {
        // Compute the msg to sign by combining UpdateAction with sequence no
        let sig_hash = action.compute_sighash(self.seqno);

        // Use the generic multisig verification function
        verify_multisig(
            &self.config,
            vote.voter_indices(),
            &sig_hash.into(),
            vote.signature(),
        )
    }

    /// Increments the nonce.
    pub fn increment_seqno(&mut self) {
        self.seqno += 1;
    }

    /// Get the current sequence number (for testing).
    #[cfg(test)]
    pub fn seqno(&self) -> u64 {
        self.seqno
    }
}
