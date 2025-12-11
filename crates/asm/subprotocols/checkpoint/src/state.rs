//! Checkpoint subprotocol state and configuration types.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_checkpoint_types_new::{CheckpointPayload, EpochSummary};
use strata_identifiers::{Buf32, CredRule, Epoch, L1BlockCommitment, L2BlockCommitment};
use strata_predicate::PredicateKey;

/// Checkpoint subprotocol state.
///
/// Maintains the current verified checkpoint state including the last verified
/// epoch summary, sequencer credential, and checkpoint predicate for proof verification.
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CheckpointState {
    /// Credential rule for sequencer signature verification.
    pub sequencer_cred: CredRule,

    /// Predicate for checkpoint proof verification.
    pub checkpoint_predicate: PredicateKey,

    /// Summary of the last verified checkpoint epoch.
    /// None if no checkpoint has been verified yet (pre-genesis).
    pub verified_epoch_summary: Option<EpochSummary>,

    /// L1 block commitment of the last checkpoint transaction.
    pub last_checkpoint_l1: L1BlockCommitment,
}

impl CheckpointState {
    /// Create initial state from configuration.
    pub fn new(config: &CheckpointConfig) -> Self {
        Self {
            sequencer_cred: config.sequencer_cred.clone(),
            checkpoint_predicate: config.checkpoint_predicate.clone(),
            verified_epoch_summary: None,
            last_checkpoint_l1: config.genesis_l1_block,
        }
    }

    /// Get the current verified epoch number.
    pub fn current_epoch(&self) -> Option<Epoch> {
        self.verified_epoch_summary.as_ref().map(|s| s.epoch())
    }

    /// Get the expected next epoch number.
    pub fn expected_next_epoch(&self) -> Epoch {
        match &self.verified_epoch_summary {
            Some(summary) => summary.epoch() + 1,
            None => 0,
        }
    }

    /// Check if we can accept a checkpoint for the given epoch.
    pub fn can_accept_epoch(&self, epoch: Epoch) -> bool {
        epoch == self.expected_next_epoch()
    }

    /// Get the last verified L2 block commitment (terminal block of last epoch).
    pub fn last_l2_terminal(&self) -> Option<&L2BlockCommitment> {
        self.verified_epoch_summary.as_ref().map(|s| s.terminal())
    }

    /// Get the L1 block commitment from the last verified epoch.
    ///
    /// This returns the L1 block that was referenced in the terminal block
    /// of the last verified epoch (from `EpochSummary::new_l1()`).
    pub fn epoch_l1_ref(&self) -> Option<&L1BlockCommitment> {
        self.verified_epoch_summary.as_ref().map(|s| s.new_l1())
    }

    /// Update state with a newly verified checkpoint.
    ///
    /// Builds the epoch summary from the checkpoint data combined with current state
    /// (for prev_terminal) and updates state accordingly.
    pub fn update_with_checkpoint(&mut self, checkpoint: &CheckpointPayload) {
        let batch_info = checkpoint.batch_info();
        let transition = checkpoint.transition();

        // prev_terminal comes from current state (the terminal of the previous epoch)
        // For the first checkpoint (epoch 0), this will be null/zero
        let prev_terminal = self
            .verified_epoch_summary
            .as_ref()
            .map(|s| *s.terminal())
            .unwrap_or_else(L2BlockCommitment::null);

        let epoch_summary = EpochSummary::new(
            batch_info.epoch(),
            *batch_info.final_l2_block(), // terminal: this epoch's final L2 block
            prev_terminal,                // prev_terminal: from current state
            *batch_info.final_l1_block(), // new_l1: this epoch's final L1 block
            *transition.post_state_root(), // final_state: post-execution state
        );

        self.verified_epoch_summary = Some(epoch_summary);
        self.last_checkpoint_l1 = *batch_info.final_l1_block();
    }

    /// Update the sequencer credential.
    pub fn update_sequencer_cred(&mut self, new_key: Buf32) {
        self.sequencer_cred = CredRule::SchnorrKey(new_key);
    }

    /// Update the checkpoint predicate.
    pub fn update_checkpoint_predicate(&mut self, new_predicate: PredicateKey) {
        self.checkpoint_predicate = new_predicate;
    }
}

/// Configuration parameters for checkpoint subprotocol initialization.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointConfig {
    /// Initial sequencer credential for signature verification.
    pub sequencer_cred: CredRule,

    /// Initial checkpoint predicate for proof verification.
    pub checkpoint_predicate: PredicateKey,

    /// Genesis L1 block commitment (starting point for L1 height validation).
    pub genesis_l1_block: L1BlockCommitment,
}
