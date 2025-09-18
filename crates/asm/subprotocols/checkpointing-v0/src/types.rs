//! Checkpointing v0 compatible data structures
//!
//! This module defines data structures that maintain compatibility with the current
//! checkpoint implementation while incorporating SPS-62 concepts where applicable.
//!
//! NOTE: This is checkpointing v0 which focuses on feature parity with the current
//! checkpointing system. Future versions will be fully SPS-62 compatible.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
// Re-export current checkpoint types for compatibility
pub use strata_primitives::batch::{
    BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar, SignedCheckpoint,
};
use strata_primitives::{
    batch::Checkpoint as PrimitivesCheckpoint, buf::Buf32, l1::L1BlockCommitment,
};

/// Internal TxFilterConfigTransition to remove dependency
///
/// NOTE: This is a temporary internal structure to avoid depending on
/// TxFilterConfigTransition from primitives until it's removed from the circuit
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct InternalTxFilterConfigTransition {
    /// Hash of the TxFilterConfig before the transition
    pub pre_config_hash: Buf32,
    /// Hash of the TxFilterConfig after the transition
    pub post_config_hash: Buf32,
}

/// Internal BatchTransition without external TxFilterConfigTransition dependency
///
/// NOTE: This mirrors the current BatchTransition but uses our internal
/// TxFilterConfigTransition to remove the dependency until circuit changes
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct InternalBatchTransition {
    /// Epoch number
    pub epoch: u64,
    /// Chainstate root transition
    pub chainstate_transition: strata_state::batch::ChainstateRootTransition,
    /// TX filter config transition (internal)
    pub tx_filters_transition: InternalTxFilterConfigTransition,
}

/// Convert from current BatchTransition to internal format
impl From<BatchTransition> for InternalBatchTransition {
    fn from(bt: BatchTransition) -> Self {
        Self {
            epoch: bt.epoch,
            chainstate_transition: bt.chainstate_transition,
            tx_filters_transition: InternalTxFilterConfigTransition {
                pre_config_hash: bt.tx_filters_transition.pre_config_hash,
                post_config_hash: bt.tx_filters_transition.post_config_hash,
            },
        }
    }
}

/// Convert from internal format to current BatchTransition
impl From<InternalBatchTransition> for BatchTransition {
    fn from(ibt: InternalBatchTransition) -> Self {
        Self {
            epoch: ibt.epoch,
            chainstate_transition: ibt.chainstate_transition,
            tx_filters_transition: strata_primitives::batch::TxFilterConfigTransition {
                pre_config_hash: ibt.tx_filters_transition.pre_config_hash,
                post_config_hash: ibt.tx_filters_transition.post_config_hash,
            },
        }
    }
}

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

/// Auxiliary input for checkpoint verification (simplified for v0)
///
/// NOTE: This is a simplified version compared to full SPS-62 auxiliary input.
/// Future versions will implement complete L1 oracle functionality.
#[derive(Clone, Debug, Default, BorshSerialize, BorshDeserialize)]
pub struct CheckpointV0AuxInput {
    /// Current L1 block information (simplified)
    pub current_l1_height: u64,
    pub current_l1_blkid: Buf32,
}

/// Withdrawal message extraction result
///
/// Contains structured withdrawal intents extracted from checkpoint chainstate
/// that are ready to be forwarded to the bridge subprotocol for processing.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct WithdrawalMessages {
    /// Parsed withdrawal intents from the chainstate pending_withdraws queue
    pub intents: Vec<WithdrawalIntentData>,
    /// Number of withdrawal messages found
    pub count: usize,
}

/// Withdrawal intent data extracted from chainstate
///
/// This mirrors the `bridge_ops::WithdrawalIntent` structure but avoids
/// importing the full state crate dependencies in the ASM subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct WithdrawalIntentData {
    /// Amount in satoshis
    pub amount_sats: u64,
    /// Destination Bitcoin descriptor bytes
    pub destination_bytes: Vec<u8>,
    /// L2 transaction ID that initiated the withdrawal
    pub withdrawal_txid: Buf32,
}

impl WithdrawalMessages {
    /// Create empty withdrawal messages
    pub fn empty() -> Self {
        Self {
            intents: Vec::new(),
            count: 0,
        }
    }

    /// Create withdrawal messages from intent data
    pub fn from_intents(intents: Vec<WithdrawalIntentData>) -> Self {
        let count = intents.len();
        Self { intents, count }
    }

    /// Extract withdrawal messages from checkpoint sidecar
    ///
    /// Parses the checkpoint sidecar to extract withdrawal requests that need to be
    /// forwarded to the bridge subprotocol for L2→L1 withdrawals.
    ///
    /// # Arguments
    /// * `sidecar` - The checkpoint sidecar containing serialized chainstate data
    ///
    /// # Returns
    /// `WithdrawalMessages` containing extracted withdrawal intents, or empty if extraction fails
    ///
    /// # Implementation Status
    /// Currently returns empty as this requires a public accessor method on Chainstate
    /// for the `pending_withdraws` field. The chainstate deserialization works correctly.
    ///
    /// # TODO
    /// This function needs `strata_state::chain_state::Chainstate` to expose a public
    /// getter method like `pub fn pending_withdraws(&self) -> &StateQueue<WithdrawalIntent>`
    /// to enable extraction of withdrawal intents from checkpoint sidecars.
    pub fn from_checkpoint_sidecar(sidecar: &CheckpointSidecar) -> Self {
        use strata_asm_common::logging;

        // Verify chainstate deserialization works (this validates the approach)
        let chainstate_bytes = sidecar.chainstate();
        match borsh::from_slice::<strata_state::chain_state::Chainstate>(chainstate_bytes) {
            Ok(_chainstate) => {
                // Chainstate deserialization successful - withdrawal extraction is feasible
                // but blocked by private field access

                logging::info!("Chainstate deserialized successfully from checkpoint sidecar");
                logging::warn!("Withdrawal extraction requires public accessor for Chainstate::pending_withdraws");

                // TODO: Once Chainstate exposes pending_withdraws() method:
                // let pending_withdrawals = chainstate.pending_withdraws();
                // let withdrawal_entries = pending_withdrawals.entries();
                //
                // let mut withdrawal_intents = Vec::with_capacity(withdrawal_entries.len());
                // for intent in withdrawal_entries {
                //     withdrawal_intents.push(WithdrawalIntentData {
                //         amount_sats: intent.amt().to_sat(),
                //         destination_bytes: intent.destination().to_bytes(),
                //         withdrawal_txid: *intent.withdrawal_txid(),
                //     });
                // }
                // return Self::from_intents(withdrawal_intents);
            }
            Err(e) => {
                logging::warn!(
                    "Failed to deserialize chainstate from checkpoint sidecar: {:?}",
                    e
                );
            }
        }

        Self::empty()
    }
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
