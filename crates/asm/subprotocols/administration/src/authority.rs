use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_administration_txs::actions::MultisigAction;
use strata_crypto::multisig::{
    MultisigError, SchnorrMultisigConfig, SchnorrMultisigSignature, verify_multisig,
};
use strata_primitives::roles::Role;

/// Manages multisignature operations for a given role and key set, with replay protection via a
/// seqno.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    role: Role,
    /// The public keys of all grant-holders authorized to sign.
    config: SchnorrMultisigConfig,
    /// Sequence number for the multisig configuration. It increases on each valid action.
    /// This is used to prevent replay attacks.
    seqno: u64,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: SchnorrMultisigConfig) -> Self {
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
    pub fn config(&self) -> &SchnorrMultisigConfig {
        &self.config
    }

    /// Mutably borrow the multisig configuration.
    pub fn config_mut(&mut self) -> &mut SchnorrMultisigConfig {
        &mut self.config
    }

    /// Verify that `signature` is a valid threshold signature for `action` under the current config
    /// and seqno.
    ///
    /// Uses the generic multisig verification function to orchestrate the workflow.
    pub fn verify_action_signature(
        &self,
        action: &MultisigAction,
        signature: &SchnorrMultisigSignature,
    ) -> Result<(), MultisigError> {
        // Compute the msg to sign by combining UpdateAction with sequence no
        let sig_hash = action.compute_sighash(self.seqno);

        // Use the generic multisig verification function
        verify_multisig(&self.config, signature, &sig_hash.into())
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
