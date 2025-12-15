//! Checkpoint subprotocol state and configuration types.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_checkpoint_types_ssz::{CheckpointPayload, EpochSummary, L1Commitment};
use strata_identifiers::{Buf32, CredRule, Epoch, OLBlockCommitment};
use strata_predicate::PredicateKey;

/// Checkpoint subprotocol state.
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CheckpointState {
    /// Credential rule for sequencer signature verification.
    /// Updated via `UpdateSequencerKey` message from admin subprotocol.
    sequencer_cred: CredRule,

    /// Predicate for checkpoint ZK proof verification.
    /// Updated via `UpdateCheckpointPredicate` message from admin subprotocol.
    checkpoint_predicate: PredicateKey,

    /// Summary of the last verified checkpoint epoch.
    /// `None` before the first checkpoint is verified.
    verified_epoch_summary: Option<EpochSummary>,

    /// Last L1 commitment covered by checkpoints (seeded from genesis).
    last_covered_l1: L1Commitment,
}

impl CheckpointState {
    /// Create initial state from configuration.
    pub fn new(config: &CheckpointConfig) -> Self {
        Self {
            sequencer_cred: config.sequencer_cred.clone(),
            checkpoint_predicate: config.checkpoint_predicate.clone(),
            verified_epoch_summary: None,
            last_covered_l1: config.genesis_l1,
        }
    }

    /// Returns the sequencer credential rule.
    pub fn sequencer_cred(&self) -> &CredRule {
        &self.sequencer_cred
    }

    /// Returns the checkpoint predicate for proof verification.
    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }

    /// Returns the verified epoch summary, if any.
    pub fn verified_epoch_summary(&self) -> Option<&EpochSummary> {
        self.verified_epoch_summary.as_ref()
    }

    /// Get the expected next epoch number.
    pub fn expected_next_epoch(&self) -> Epoch {
        self.verified_epoch_summary
            .as_ref()
            .map(|s| s.epoch() + 1)
            .unwrap_or(0)
    }

    /// Update the sequencer credential.
    pub fn update_sequencer_cred(&mut self, new_key: Buf32) {
        self.sequencer_cred = CredRule::SchnorrKey(new_key);
    }

    /// Update the checkpoint predicate.
    pub fn update_checkpoint_predicate(&mut self, new_predicate: PredicateKey) {
        self.checkpoint_predicate = new_predicate;
    }

    /// Returns the height of the last L1 block covered by the previous checkpoint.
    pub fn last_covered_l1_height(&self) -> u32 {
        self.last_covered_l1.height
    }

    /// Returns the last L1 block commitment covered by the previous checkpoint.
    pub fn last_covered_l1(&self) -> L1Commitment {
        self.last_covered_l1
    }

    /// Returns the slot of the last L2 terminal block.
    /// Returns `None` before the first checkpoint is verified.
    pub fn last_l2_terminal_slot(&self) -> Option<u64> {
        self.verified_epoch_summary
            .as_ref()
            .map(|s| s.terminal().slot())
    }

    /// Returns the last L2 terminal block commitment, if any.
    pub fn last_l2_terminal(&self) -> Option<OLBlockCommitment> {
        self.verified_epoch_summary
            .as_ref()
            .map(|s| *s.terminal())
    }

    /// Returns the pre-state root for the next checkpoint.
    ///
    /// This is the final state from the last verified epoch, or zero for the first checkpoint.
    pub fn pre_state_root(&self) -> Buf32 {
        self.verified_epoch_summary
            .as_ref()
            .map(|s| *s.final_state())
            .unwrap_or_else(Buf32::zero)
    }

    /// Update state with a verified checkpoint.
    ///
    /// Called after the checkpoint signature and proof have been verified.
    pub fn update_with_checkpoint(&mut self, checkpoint: &CheckpointPayload) {
        let batch_info = &checkpoint.commitment.batch_info;
        let transition = &checkpoint.commitment.transition;

        // prev_terminal comes from current state (the terminal of the previous epoch).
        // For the first checkpoint (epoch 0), this will be null/zero.
        let prev_terminal = self
            .verified_epoch_summary
            .as_ref()
            .map(|s| *s.terminal())
            .unwrap_or_else(OLBlockCommitment::null);

        let epoch_summary = EpochSummary::new(
            batch_info.epoch,
            batch_info.l2_range.end, // terminal: this epoch's final L2 block
            prev_terminal,           // prev_terminal: from current state
            batch_info.l1_range.end, // new_l1: this epoch's final L1 block
            transition.post_state_root, // final_state: post-execution state
        );

        self.verified_epoch_summary = Some(epoch_summary);
        self.last_covered_l1 = batch_info.l1_range.end;

    }
}

/// Configuration parameters for checkpoint subprotocol initialization.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointConfig {
    /// Initial sequencer credential for signature verification.
    pub sequencer_cred: CredRule,

    /// Initial checkpoint predicate for proof verification.
    pub checkpoint_predicate: PredicateKey,

    /// Genesis L1 block commitment.
    pub genesis_l1: L1Commitment,
}
