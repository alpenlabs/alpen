use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::CheckpointConfig;
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_identifiers::L2BlockCommitment;
use strata_predicate::PredicateKey;

/// Checkpoint subprotocol state.
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CheckpointState {
    /// Predicate for sequencer signature verification.
    /// Updated via `UpdateSequencerKey` message from admin subprotocol.
    pub sequencer_predicate: PredicateKey,

    /// Predicate for checkpoint ZK proof verification.
    /// Updated via `UpdateCheckpointPredicate` message from admin subprotocol.
    pub checkpoint_predicate: PredicateKey,

    /// Last verified checkpoint tip position.
    /// Tracks the OL state that has been proven and verified by ASM.
    pub verified_tip: CheckpointTip,
}

impl CheckpointState {
    /// Initializes checkpoint state from configuration.
    pub fn init(config: CheckpointConfig) -> Self {
        let genesis_epoch = 0;
        let genesis_l2_slot = 0;
        let genesis_l2_commitment =
            L2BlockCommitment::new(genesis_l2_slot, config.genesis_ol_blkid);
        let genesis_tip = CheckpointTip::new(
            genesis_epoch,
            config.genesis_l1_height,
            genesis_l2_commitment,
        );
        Self::new(
            config.sequencer_predicate,
            config.checkpoint_predicate,
            genesis_tip,
        )
    }

    /// Returns the sequencer predicate for signature verification.
    pub(crate) fn new(
        sequencer_predicate: PredicateKey,
        checkpoint_predicate: PredicateKey,
        verified_tip: CheckpointTip,
    ) -> Self {
        Self {
            sequencer_predicate,
            checkpoint_predicate,
            verified_tip,
        }
    }

    /// Returns the sequencer predicate for signature verification.
    pub fn sequencer_predicate(&self) -> &PredicateKey {
        &self.sequencer_predicate
    }

    /// Returns the checkpoint predicate for proof verification.
    pub fn checkpoint_predicate(&self) -> &PredicateKey {
        &self.checkpoint_predicate
    }

    /// Returns the last verified checkpoint tip.
    pub fn verified_tip(&self) -> &CheckpointTip {
        &self.verified_tip
    }

    /// Update the sequencer predicate with a new Schnorr public key.
    pub(crate) fn update_sequencer_predicate(&mut self, new_predicate: PredicateKey) {
        self.sequencer_predicate = new_predicate
    }

    /// Update the checkpoint predicate.
    pub(crate) fn update_checkpoint_predicate(&mut self, new_predicate: PredicateKey) {
        self.checkpoint_predicate = new_predicate;
    }

    /// Updates the verified checkpoint tip after successful verification.
    pub(crate) fn update_verified_tip(&mut self, new_tip: CheckpointTip) {
        self.verified_tip = new_tip
    }
}
