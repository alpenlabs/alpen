//! Checkpoint v0 data structures.

use ssz::{Decode, Encode};
use strata_checkpoint_types::Checkpoint;
use strata_identifiers::Epoch;
use strata_predicate::PredicateKey;
use strata_primitives::{L1Height, block_credential::CredRule, buf::Buf32, l1::L1BlockCommitment};

use crate::{CheckpointV0VerifierState, CredRuleState, ssz_generated::ssz::state::PredicateBytes};

/// Checkpoint verifier state for checkpoint v0
///
/// NOTE: This maintains state similar to the current core subprotocol but
/// simplified for checkpoint v0 compatibility
/// Verification parameters for checkpoint v0
///
/// NOTE: This bridges to the current verification system while maintaining
/// some SPS-62 concepts for future compatibility.
/// Configuration parameters don't need serialization - they're provided at init.
#[derive(Clone, Debug)]
pub struct CheckpointV0VerificationParams {
    /// Genesis L1 block commitment
    pub genesis_l1_block: L1BlockCommitment,

    /// Credential rule governing signature verification
    pub cred_rule: CredRule,

    /// Predicate used to verify the validity of the checkpoint
    pub predicate: PredicateKey,
}

/// Compatibility functions for working with current checkpoint types
impl CheckpointV0VerifierState {
    fn encode_checkpoint(checkpoint: Checkpoint) -> Vec<u8> {
        checkpoint.to_borsh_bytes()
    }

    fn decode_checkpoint(bytes: &[u8]) -> Checkpoint {
        Checkpoint::from_borsh_bytes(bytes).expect("checkpoint-v0 checkpoint bytes stay valid")
    }

    fn encode_cred_rule(cred_rule: CredRule) -> CredRuleState {
        match cred_rule {
            CredRule::Unchecked => CredRuleState {
                has_schnorr_key: false,
                schnorr_key: [0; 32].into(),
            },
            CredRule::SchnorrKey(key) => CredRuleState {
                has_schnorr_key: true,
                schnorr_key: key.into(),
            },
        }
    }

    fn decode_cred_rule(cred_rule: &CredRuleState) -> CredRule {
        if cred_rule.has_schnorr_key {
            let key_bytes: [u8; 32] = cred_rule
                .schnorr_key
                .as_ref()
                .try_into()
                .expect("checkpoint-v0 schnorr key must remain 32 bytes");
            CredRule::SchnorrKey(key_bytes.into())
        } else {
            CredRule::Unchecked
        }
    }

    fn encode_predicate(predicate: PredicateKey) -> PredicateBytes {
        PredicateBytes::new(predicate.as_ssz_bytes())
            .expect("checkpoint-v0 predicate must stay within SSZ bounds")
    }

    fn decode_predicate(bytes: &[u8]) -> PredicateKey {
        PredicateKey::from_ssz_bytes(bytes)
            .expect("checkpoint-v0 predicate bytes must remain valid")
    }

    /// Initialize from genesis parameters
    pub fn new(params: &CheckpointV0VerificationParams) -> Self {
        Self {
            has_last_checkpoint: false,
            last_checkpoint: vec![].into(),
            last_checkpoint_l1_height: params.genesis_l1_block.height().into(),
            current_verified_epoch: 0,
            cred_rule: Self::encode_cred_rule(params.cred_rule.clone()),
            predicate: Self::encode_predicate(params.predicate.clone()),
        }
    }

    /// Returns the last verified checkpoint, if one exists.
    pub fn last_checkpoint(&self) -> Option<Checkpoint> {
        self.has_last_checkpoint
            .then(|| Self::decode_checkpoint(&self.last_checkpoint))
    }

    /// Returns the active credential rule.
    pub fn cred_rule(&self) -> CredRule {
        Self::decode_cred_rule(&self.cred_rule)
    }

    /// Returns the predicate used to verify checkpoint proofs.
    pub fn predicate(&self) -> PredicateKey {
        Self::decode_predicate(&self.predicate)
    }

    /// Update state with a newly verified checkpoint
    pub fn update_with_checkpoint(&mut self, checkpoint: Checkpoint, l1_height: L1Height) {
        let epoch = checkpoint.batch_info().epoch();
        self.has_last_checkpoint = true;
        self.last_checkpoint = Self::encode_checkpoint(checkpoint).into();
        self.last_checkpoint_l1_height = l1_height.into();
        self.current_verified_epoch = epoch.into();
    }

    /// Get the latest verified epoch
    pub fn current_epoch(&self) -> Epoch {
        self.current_verified_epoch
            .try_into()
            .expect("verified epoch must fit in Epoch")
    }

    /// Get the epoch value we expect for the next checkpoint.
    pub fn expected_next_epoch(&self) -> Epoch {
        match self.has_last_checkpoint {
            true => (self.current_verified_epoch + 1)
                .try_into()
                .expect("next verified epoch must fit in Epoch"),
            false => 0,
        }
    }

    /// Check if we can accept a checkpoint for the given epoch
    ///
    /// Returns `true` if the epoch is exactly one greater than the current verified epoch.
    /// This enforces sequential epoch progression without gaps.
    ///
    /// # Arguments
    /// * `epoch` - The epoch number to validate
    ///
    /// # Returns
    /// `true` if the epoch can be accepted, `false` otherwise
    pub fn can_accept_epoch(&self, epoch: Epoch) -> bool {
        epoch == self.expected_next_epoch()
    }

    /// Update the sequencer public key used to validate checkpoint signatures.
    pub fn update_sequencer_key(&mut self, new_pubkey: Buf32) {
        self.cred_rule = Self::encode_cred_rule(CredRule::SchnorrKey(new_pubkey));
    }

    /// Update the rollup verifying key used for proof verification.
    pub fn update_predicate(&mut self, new_predicate: PredicateKey) {
        self.predicate = Self::encode_predicate(new_predicate);
    }
}
