use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_txs_admin::actions::MultisigAction;
use strata_crypto::threshold_signature::{
    SignatureSet, ThresholdConfig, ThresholdSignatureError, verify_threshold_signatures,
};
use strata_primitives::roles::Role;

/// Manages threshold signature operations for a given role and key set, with replay protection via
/// a seqno.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct MultisigAuthority {
    /// The role of this threshold signature authority.
    role: Role,
    /// The public keys of all grant-holders authorized to sign.
    config: ThresholdConfig,
    /// Sequence number for the threshold configuration. It increases on each valid action.
    /// This is used to prevent replay attacks.
    seqno: u64,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: ThresholdConfig) -> Self {
        Self {
            role,
            config,
            seqno: 0,
        }
    }

    /// The role authorized to perform threshold signature operations.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Borrow the current threshold configuration.
    pub fn config(&self) -> &ThresholdConfig {
        &self.config
    }

    /// Mutably borrow the threshold configuration.
    pub fn config_mut(&mut self) -> &mut ThresholdConfig {
        &mut self.config
    }

    /// Verify that `signatures` is a valid threshold signature set for `action` under the current
    /// config and seqno.
    ///
    /// Uses individual ECDSA signature verification against each signer's public key.
    pub fn verify_action_signature(
        &self,
        action: &MultisigAction,
        signatures: &SignatureSet,
    ) -> Result<(), ThresholdSignatureError> {
        // Compute the msg to sign by combining UpdateAction with sequence no
        let sig_hash = action.compute_sighash(self.seqno);

        // Verify each ECDSA signature against the corresponding public key
        verify_threshold_signatures(&self.config, signatures.signatures(), &sig_hash.into())
    }

    /// Increments the seqno.
    pub fn increment_seqno(&mut self) {
        self.seqno += 1;
    }

    /// Get the current sequence number (for testing).
    #[cfg(test)]
    pub fn seqno(&self) -> u64 {
        self.seqno
    }
}
