//! State management for the Core subprotocol
//!
//! This module contains the state structures and management logic for the Core subprotocol.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::L2ToL1Msg;
use strata_checkpoint_types::{Checkpoint, EpochSummary};
use strata_predicate::PredicateKey;
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockCommitment, L1BlockId},
};

use crate::error::*;

/// OL Core state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CoreOLState {
    /// The rollup verifying key used to verify each new checkpoint proof
    /// that has been posted on Bitcoin.
    pub checkpoint_predicate: PredicateKey,

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
    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }

    /// Set the rollup verifying key by serializing to bytes
    pub fn set_checkpoint_predicate(&mut self, predicate: PredicateKey) -> Result<()> {
        // TODO: Add validation for the verifying key
        self.checkpoint_predicate = predicate;
        Ok(())
    }
}

/// Applies a validated checkpoint to the current state
///
/// This function updates the Core subprotocol state with the new checkpoint
/// information. It should only be called after all validation has passed.
///
/// # Arguments
/// * `state` - Mutable reference to the current state
/// * `new_epoch_summary` - The new epoch summary to apply
/// * `checkpoint` - The checkpoint containing the final L1 block reference
pub(crate) fn apply_checkpoint_to_state(
    state: &mut CoreOLState,
    new_epoch_summary: EpochSummary,
    checkpoint: &Checkpoint,
) {
    state.verified_checkpoint = new_epoch_summary;
    state.last_checkpoint_ref = *checkpoint.batch_info().final_l1_block().blkid();
}

/// Genesis configuration for the Core subprotocol.
///
/// This configuration contains only the essential data that is known at genesis time.
/// Other fields like the initial EpochSummary are constructed during initialization
/// since most of their values are either zero or unknowable until genesis processing.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CoreGenesisConfig {
    /// The initial checkpoint verifying key for zk-SNARK proof verification
    pub checkpoint_predicate: PredicateKey,

    /// The L1 block commitment where ASM genesis occurs
    /// This includes both the block ID and height, providing complete L1 block information
    pub genesis_l1_block: L1BlockCommitment,

    /// The authorized sequencer's public key for checkpoint submission
    pub sequencer_pubkey: Buf32,
}

impl CoreGenesisConfig {
    /// Create a new genesis config with the given rollup verifying key
    pub fn new(
        checkpoint_predicate: PredicateKey,
        genesis_l1_block: L1BlockCommitment,
        sequencer_pubkey: Buf32,
    ) -> Result<Self> {
        Ok(Self {
            checkpoint_predicate,
            genesis_l1_block,
            sequencer_pubkey,
        })
    }
}

/// [PLACE_HOLDER] => Finalize the public parameters for checkpoint proof
#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct CheckpointProofPublicParameters {
    /// New epoch summary.
    pub epoch_summary: EpochSummary,
    /// Hash of the OL state diff.
    pub state_diff_hash: Buf32,
    /// Ordered messages L2 → L1. For now, this only includes the
    /// withdrawal requests.
    pub l2_to_l1_msgs: Vec<L2ToL1Msg>,
    /// Previous L1 commitment or genesis.
    pub prev_l1_ref: L1BlockCommitment,
    /// Commitment to the range of L1 → L2 messages.
    pub l1_to_l2_msgs_range_commitment_hash: Buf32,
}
