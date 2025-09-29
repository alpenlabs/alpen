//! Checkpointing v0 data structures
//!
//! This module defines data structures that maintain compatibility with the current
//! checkpoint implementation while incorporating SPS-62 concepts where applicable.
//!
//! NOTE: This is checkpointing v0 which focuses on feature parity with the current
//! checkpointing system. Future versions will be fully SPS-62 compatible.

use borsh::{BorshDeserialize, BorshSerialize};
// Re-export current checkpoint types for compatibility
use strata_primitives::{
    batch::Checkpoint as PrimitivesCheckpoint, block_credential::CredRule, buf::Buf32,
    l1::L1BlockCommitment, proof::RollupVerifyingKey,
};

/// Checkpoint verifier state for checkpointing v0
///
/// NOTE: This maintains state similar to the current core subprotocol but
/// simplified for checkpointing v0 compatibility
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CheckpointV0VerifierState {
    /// The last verified checkpoint
    pub last_checkpoint: Option<PrimitivesCheckpoint>,
    /// Last L1 block where we got a valid checkpoint
    pub last_checkpoint_l1_height: u64,
    /// Current epoch we've verified up to
    pub current_verified_epoch: u64,
    /// Credential rule governing signature verification
    pub cred_rule: CredRule,
    /// Rollup verifying key used for proof verification
    pub rollup_verifying_key: RollupVerifyingKey,
}

/// Verification parameters for checkpointing v0
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
    /// Rollup verifying key for proof verification
    pub rollup_verifying_key: RollupVerifyingKey,
}

/// Compatibility functions for working with current checkpoint types
impl CheckpointV0VerifierState {
    /// Initialize from genesis parameters
    pub fn new(params: &CheckpointV0VerificationParams) -> Self {
        Self {
            last_checkpoint: None,
            last_checkpoint_l1_height: params.genesis_l1_block.height(),
            current_verified_epoch: 0,
            cred_rule: params.cred_rule.clone(),
            rollup_verifying_key: params.rollup_verifying_key.clone(),
        }
    }

    /// Update state with a newly verified checkpoint
    pub fn update_with_checkpoint(&mut self, checkpoint: PrimitivesCheckpoint, l1_height: u64) {
        let epoch = checkpoint.batch_info().epoch();
        self.last_checkpoint = Some(checkpoint);
        self.last_checkpoint_l1_height = l1_height;
        self.current_verified_epoch = epoch;
    }

    /// Get the latest verified epoch
    pub fn current_epoch(&self) -> u64 {
        self.current_verified_epoch
    }

    /// Get the epoch value we expect for the next checkpoint.
    pub fn expected_next_epoch(&self) -> u64 {
        match &self.last_checkpoint {
            Some(_) => self.current_verified_epoch + 1,
            None => 0,
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
    pub fn can_accept_epoch(&self, epoch: u64) -> bool {
        epoch == self.expected_next_epoch()
    }

    /// Update the sequencer public key used to validate checkpoint signatures.
    pub fn update_sequencer_key(&mut self, new_pubkey: Buf32) {
        self.cred_rule = CredRule::SchnorrKey(new_pubkey);
    }

    /// Update the rollup verifying key used for proof verification.
    pub fn update_rollup_verifying_key(&mut self, new_vk: RollupVerifyingKey) {
        self.rollup_verifying_key = new_vk;
    }
}
