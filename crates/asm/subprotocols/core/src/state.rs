//! State management for the Core subprotocol
//!
//! This module contains the state structures and management logic for the Core subprotocol.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{
    batch::EpochSummary, buf::Buf32, l1::L1BlockId, proof::RollupVerifyingKey,
};

use crate::error::*;

/// OL Core state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CoreOLState {
    /// The rollup verifying key used to verify each new checkpoint proof
    /// that has been posted on Bitcoin. Stored as serialized bytes for Borsh compatibility.
    pub checkpoint_vk_bytes: Vec<u8>,

    /// Summary of the last checkpoint that was successfully verified.
    /// New proofs are checked against this epoch summary.
    pub verified_checkpoint: EpochSummary,

    /// The L1 block ID up to which the `verified_checkpoint` covers.
    pub last_checkpoint_ref: L1BlockId,

    /// Public key of the sequencer authorized to submit checkpoint proofs.
    pub sequencer_pubkey: Buf32,
}

impl CoreOLState {
    /// Get the rollup verifying key by deserializing from stored bytes
    pub fn checkpoint_vk(&self) -> std::result::Result<RollupVerifyingKey, CoreError> {
        serde_json::from_slice(&self.checkpoint_vk_bytes)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))
    }

    /// Set the rollup verifying key by serializing to bytes
    pub fn set_checkpoint_vk(
        &mut self,
        vk: &RollupVerifyingKey,
    ) -> std::result::Result<(), CoreError> {
        self.checkpoint_vk_bytes = serde_json::to_vec(vk)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))?;
        Ok(())
    }
}

/// Genesis configuration for the Core subprotocol.
///
/// This structure contains all necessary parameters to properly initialize
/// the Core subprotocol state.
///
/// This struct sharing the same fields as CoreOLState but i create this
/// separately to avoid confusion (for now).
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CoreGenesisConfig {
    /// The initial checkpoint verifying key for zk-SNARK proof verification
    /// Stored as serialized bytes for Borsh compatibility.
    pub checkpoint_vk_bytes: Vec<u8>,

    /// The initial verified checkpoint state (usually genesis checkpoint)
    pub initial_checkpoint: EpochSummary,

    /// The initial L1 block reference for the checkpoint
    pub initial_l1_ref: L1BlockId,

    /// The authorized sequencer's public key for checkpoint submission
    pub sequencer_pubkey: Buf32,
}

impl CoreGenesisConfig {
    /// Create a new genesis config with the given rollup verifying key
    pub fn new(
        checkpoint_vk: &RollupVerifyingKey,
        initial_checkpoint: EpochSummary,
        initial_l1_ref: L1BlockId,
        sequencer_pubkey: Buf32,
    ) -> std::result::Result<Self, CoreError> {
        let checkpoint_vk_bytes = serde_json::to_vec(checkpoint_vk)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))?;

        Ok(Self {
            checkpoint_vk_bytes,
            initial_checkpoint,
            initial_l1_ref,
            sequencer_pubkey,
        })
    }

    /// Get the rollup verifying key by deserializing from stored bytes
    pub fn checkpoint_vk(&self) -> std::result::Result<RollupVerifyingKey, CoreError> {
        serde_json::from_slice(&self.checkpoint_vk_bytes)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))
    }
}
