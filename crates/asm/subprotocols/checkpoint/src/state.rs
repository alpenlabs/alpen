//! Checkpoint subprotocol state and configuration types.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_checkpoint_types_ssz::{CheckpointPayload, EpochSummary, L1Commitment};
use strata_identifiers::{Buf32, Epoch, OLBlockCommitment};
use strata_predicate::{PredicateKey, PredicateTypeId};

/// Checkpoint subprotocol state.
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CheckpointState {
    /// Predicate for sequencer signature verification.
    /// Updated via `UpdateSequencerKey` message from admin subprotocol.
    sequencer_predicate: PredicateKey,

    /// Predicate for checkpoint ZK proof verification.
    /// Updated via `UpdateCheckpointPredicate` message from admin subprotocol.
    checkpoint_predicate: PredicateKey,

    /// Summary of the current epoch state.
    ///
    /// Before the first checkpoint is verified, this contains a genesis summary with:
    /// - `epoch` = 0 (first expected epoch)
    /// - `terminal` = null (sentinel indicating no epoch verified yet)
    /// - `l1_end` = genesis L1 commitment
    ///
    /// After checkpoints are verified, this contains the last verified epoch's data.
    epoch_summary: EpochSummary,
}

impl CheckpointState {
    /// Create initial state from configuration.
    ///
    /// Initializes with a genesis epoch summary where:
    /// - `epoch` = 0 (first expected epoch)
    /// - `terminal` = null (indicates no epoch has been verified yet)
    /// - `l1_end` = genesis L1 commitment from config
    /// - `final_state` = genesis OL state root (pre-state for first checkpoint)
    pub fn new(config: &CheckpointConfig) -> Self {
        let genesis_summary = EpochSummary::new(
            0,                            // First expected epoch
            OLBlockCommitment::null(),    // Null terminal = genesis sentinel
            OLBlockCommitment::null(),    // No previous terminal
            config.genesis_l1,            // Genesis L1 commitment
            config.genesis_ol_state_root, // Genesis OL state root
        );
        Self {
            sequencer_predicate: config.sequencer_predicate.clone(),
            checkpoint_predicate: config.checkpoint_predicate.clone(),
            epoch_summary: genesis_summary,
        }
    }

    /// Returns true if no checkpoint has been verified yet (genesis state).
    ///
    /// Genesis detection uses a sentinel value: the terminal block commitment is set to
    /// [`OLBlockCommitment::null()`] (zero blkid) during initialization. This is safe because:
    /// - No real L2 block can have a zero hash (cryptographically infeasible)
    /// - The sentinel is consistently set via [`OLBlockCommitment::null()`] constructor
    /// - After the first checkpoint is verified, terminal is set to the actual L2 block
    pub fn is_genesis(&self) -> bool {
        self.epoch_summary.terminal() == &OLBlockCommitment::null()
    }

    /// Returns the sequencer predicate for signature verification.
    pub fn sequencer_predicate(&self) -> &PredicateKey {
        &self.sequencer_predicate
    }

    /// Returns the checkpoint predicate for proof verification.
    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }

    /// Returns the verified epoch summary, if any.
    ///
    /// Returns `None` if no checkpoint has been verified yet (genesis state).
    pub fn verified_epoch_summary(&self) -> Option<&EpochSummary> {
        if self.is_genesis() {
            None
        } else {
            Some(&self.epoch_summary)
        }
    }

    /// Get the expected next epoch number.
    pub fn expected_next_epoch(&self) -> Epoch {
        if self.is_genesis() {
            // Genesis: epoch field holds the first expected epoch
            self.epoch_summary.epoch()
        } else {
            // Normal: next epoch is current + 1
            self.epoch_summary.epoch() + 1
        }
    }

    /// Update the sequencer predicate with a new Schnorr public key.
    pub fn update_sequencer_predicate(&mut self, new_key: &[u8]) {
        self.sequencer_predicate =
            PredicateKey::new(PredicateTypeId::Bip340Schnorr, new_key.to_vec());
    }

    /// Update the checkpoint predicate.
    pub fn update_checkpoint_predicate(&mut self, new_predicate: PredicateKey) {
        self.checkpoint_predicate = new_predicate;
    }

    /// Returns the height of the last L1 block covered by the previous checkpoint.
    pub fn last_covered_l1_height(&self) -> u32 {
        self.epoch_summary.l1_end().height
    }

    /// Returns the last L1 block commitment covered by the previous checkpoint.
    pub fn last_covered_l1(&self) -> L1Commitment {
        *self.epoch_summary.l1_end()
    }

    /// Returns the slot of the last L2 terminal block.
    /// Returns `None` before the first checkpoint is verified.
    pub fn last_l2_terminal_slot(&self) -> Option<u64> {
        if self.is_genesis() {
            None
        } else {
            Some(self.epoch_summary.terminal().slot())
        }
    }

    /// Returns the last L2 terminal block commitment, if any.
    pub fn last_l2_terminal(&self) -> Option<OLBlockCommitment> {
        if self.is_genesis() {
            None
        } else {
            Some(*self.epoch_summary.terminal())
        }
    }

    /// Returns the pre-state root for the next checkpoint.
    ///
    /// This is the final state from the last verified epoch, or the genesis OL
    /// state root for the first checkpoint.
    pub fn pre_state_root(&self) -> Buf32 {
        *self.epoch_summary.final_state()
    }

    /// Update state with a verified checkpoint.
    ///
    /// Called after the checkpoint signature and proof have been verified.
    pub fn update_with_checkpoint(&mut self, checkpoint: &CheckpointPayload) {
        let batch_info = &checkpoint.commitment.batch_info;

        // prev_terminal comes from current state (the terminal of the previous epoch).
        // For the first checkpoint (epoch 0), this will be null/zero.
        let prev_terminal = if self.is_genesis() {
            OLBlockCommitment::null()
        } else {
            *self.epoch_summary.terminal()
        };

        self.epoch_summary = EpochSummary::new(
            batch_info.epoch,
            batch_info.l2_range.end, // terminal: this epoch's final L2 block
            prev_terminal,           // prev_terminal: from current state
            batch_info.l1_range.end, // l1_end: this epoch's final L1 block
            checkpoint.commitment.post_state_root, // final_state: post-execution state
        );
    }
}

/// Configuration parameters for checkpoint subprotocol initialization.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointConfig {
    /// Initial sequencer predicate for signature verification.
    pub sequencer_predicate: PredicateKey,

    /// Initial checkpoint predicate for proof verification.
    pub checkpoint_predicate: PredicateKey,

    /// Genesis L1 block commitment.
    pub genesis_l1: L1Commitment,

    /// Genesis OL chainstate root.
    ///
    /// This is the state root of the OL chainstate at genesis (slot 0),
    /// which serves as the pre-state root for the first checkpoint.
    pub genesis_ol_state_root: Buf32,
}
