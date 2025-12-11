//! Checkpoint subprotocol state and configuration types.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_checkpoint_types_ssz::{CheckpointPayload, EpochSummary};
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

#[cfg(test)]
mod tests {
    use strata_predicate::PredicateKey;
    use strata_test_utils_asm::checkpoint::{
        CheckpointFixtures, gen_checkpoint_payload, gen_l1_block_commitment,
    };

    use super::*;

    fn create_test_config() -> CheckpointConfig {
        let fixtures = CheckpointFixtures::new();
        CheckpointConfig {
            sequencer_cred: CredRule::SchnorrKey(fixtures.sequencer.public_key),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_block: gen_l1_block_commitment(100),
        }
    }

    #[test]
    fn test_checkpoint_state_new() {
        let config = create_test_config();
        let state = CheckpointState::new(&config);

        assert_eq!(state.sequencer_cred, config.sequencer_cred);
        assert_eq!(state.checkpoint_predicate, config.checkpoint_predicate);
        assert!(state.verified_epoch_summary.is_none());
        assert_eq!(state.last_checkpoint_l1, config.genesis_l1_block);
    }

    #[test]
    fn test_expected_next_epoch_initial() {
        let config = create_test_config();
        let state = CheckpointState::new(&config);

        // Initially, no epochs verified, so next expected is 0
        assert_eq!(state.expected_next_epoch(), 0);
        assert!(state.current_epoch().is_none());
    }

    #[test]
    fn test_can_accept_epoch() {
        let config = create_test_config();
        let state = CheckpointState::new(&config);

        // Initially can only accept epoch 0
        assert!(state.can_accept_epoch(0));
        assert!(!state.can_accept_epoch(1));
        assert!(!state.can_accept_epoch(5));
    }

    #[test]
    fn test_update_with_checkpoint() {
        let config = create_test_config();
        let mut state = CheckpointState::new(&config);

        // Create and apply epoch 0 checkpoint
        let payload_0 = gen_checkpoint_payload(0);
        state.update_with_checkpoint(&payload_0);

        // Verify state was updated
        assert_eq!(state.current_epoch(), Some(0));
        assert_eq!(state.expected_next_epoch(), 1);
        assert!(state.can_accept_epoch(1));
        assert!(!state.can_accept_epoch(0));
        assert!(!state.can_accept_epoch(2));

        // Verify epoch summary
        let summary = state.verified_epoch_summary.as_ref().unwrap();
        assert_eq!(summary.epoch(), 0);
        assert_eq!(summary.terminal(), payload_0.batch_info().final_l2_block());
        assert_eq!(
            summary.final_state(),
            payload_0.transition().post_state_root()
        );
    }

    #[test]
    fn test_sequential_epoch_updates() {
        let config = create_test_config();
        let mut state = CheckpointState::new(&config);

        // Apply epochs 0, 1, 2 sequentially
        for epoch in 0..3 {
            assert_eq!(state.expected_next_epoch(), epoch);

            let payload = gen_checkpoint_payload(epoch);
            state.update_with_checkpoint(&payload);

            assert_eq!(state.current_epoch(), Some(epoch));
        }

        // After epoch 2, next expected is 3
        assert_eq!(state.expected_next_epoch(), 3);
    }

    #[test]
    fn test_last_l2_terminal() {
        let config = create_test_config();
        let mut state = CheckpointState::new(&config);

        // Initially no terminal
        assert!(state.last_l2_terminal().is_none());

        // After epoch 0
        let payload_0 = gen_checkpoint_payload(0);
        state.update_with_checkpoint(&payload_0);

        let terminal = state.last_l2_terminal().unwrap();
        assert_eq!(terminal, payload_0.batch_info().final_l2_block());
    }

    #[test]
    fn test_update_sequencer_cred() {
        let config = create_test_config();
        let mut state = CheckpointState::new(&config);

        let new_keypair = CheckpointFixtures::new().sequencer;
        state.update_sequencer_cred(new_keypair.public_key);

        match &state.sequencer_cred {
            CredRule::SchnorrKey(key) => assert_eq!(*key, new_keypair.public_key),
            _ => panic!("Expected SchnorrKey"),
        }
    }

    #[test]
    fn test_update_checkpoint_predicate() {
        let config = create_test_config();
        let mut state = CheckpointState::new(&config);

        let new_predicate = PredicateKey::never_accept();
        state.update_checkpoint_predicate(new_predicate.clone());

        assert_eq!(state.checkpoint_predicate, new_predicate);
    }
}
