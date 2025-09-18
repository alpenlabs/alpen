//! Checkpointing v0 data structures
//!
//! This module defines data structures that maintain compatibility with the current
//! checkpoint implementation while incorporating SPS-62 concepts where applicable.
//!
//! NOTE: This is checkpointing v0 which focuses on feature parity with the current
//! checkpointing system. Future versions will be fully SPS-62 compatible.

use borsh::{BorshDeserialize, BorshSerialize};
// Re-export current checkpoint types for compatibility
// TODO: remove dependency of strata_primitives data structures to `TxFilterConfig`
pub use strata_primitives::batch::{
    BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar, SignedCheckpoint,
};
use strata_primitives::{
    batch::Checkpoint as PrimitivesCheckpoint, buf::Buf32, l1::L1BlockCommitment,
};

/// Checkpoint verifier state for checkpointing v0
///
/// NOTE: This maintains state similar to the current core subprotocol but
/// simplified for checkpointing v0 compatibility
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Default)]
pub struct CheckpointV0VerifierState {
    /// The last verified checkpoint
    pub last_checkpoint: Option<PrimitivesCheckpoint>,
    /// Last L1 block where we got a valid checkpoint
    pub last_checkpoint_l1_height: u64,
    /// Current epoch we've verified up to
    pub current_verified_epoch: u64,
}

/// Verification parameters for checkpointing v0
///
/// NOTE: This bridges to the current verification system while maintaining
/// some SPS-62 concepts for future compatibility.
/// Configuration parameters don't need serialization - they're provided at init.
#[derive(Clone, Debug)]
pub struct CheckpointV0VerificationParams {
    /// Sequencer public key for signature verification
    pub sequencer_pubkey: Buf32,
    /// Whether to skip proof verification for testing (current system compatibility)
    pub skip_proof_verification: bool,
    /// Genesis L1 block commitment
    pub genesis_l1_block: L1BlockCommitment,
    /// Rollup verifying key for proof verification
    /// Optional to support testing environments without proof verification
    pub rollup_verifying_key: Option<strata_primitives::proof::RollupVerifyingKey>,
}

/// Verification context for a checkpoint transaction
#[derive(Clone, Debug)]
pub struct CheckpointV0VerifyContext {
    /// Current L1 height when processing the checkpoint
    pub current_l1_height: u64,
    /// Public key that signed the checkpoint envelope transaction
    pub checkpoint_signer_pubkey: Buf32,
}

/// Compatibility functions for working with current checkpoint types
impl CheckpointV0VerifierState {
    /// Initialize from genesis parameters
    pub fn new_genesis(genesis_l1_block: L1BlockCommitment) -> Self {
        Self {
            last_checkpoint: None,
            last_checkpoint_l1_height: genesis_l1_block.height(),
            current_verified_epoch: 0,
        }
    }

    /// Update state with a newly verified checkpoint
    pub fn update_with_checkpoint(&mut self, checkpoint: PrimitivesCheckpoint, l1_height: u64) {
        let epoch = checkpoint.batch_info().epoch();
        self.last_checkpoint = Some(checkpoint);
        self.last_checkpoint_l1_height = l1_height;
        self.current_verified_epoch = epoch;
    }

    /// Get the current epoch we've verified
    pub fn current_epoch(&self) -> u64 {
        self.current_verified_epoch
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
        epoch == self.current_verified_epoch + 1
    }
}
