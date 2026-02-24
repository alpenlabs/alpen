use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_bridge_msgs::WithdrawOutput;
use strata_asm_params::CheckpointConfig;
use strata_btc_types::BitcoinAmount;
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

    /// Cumulative available deposit value in satoshis.
    ///
    /// Tracks the total deposit value that has been processed by the bridge but not yet
    /// consumed by withdrawal dispatches. Used to reject checkpoints whose withdrawal
    /// intents exceed the available deposit backing.
    available_deposit_sum: u64,
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

    /// Creates a new checkpoint state with the given predicates and tip.
    pub(crate) fn new(
        sequencer_predicate: PredicateKey,
        checkpoint_predicate: PredicateKey,
        verified_tip: CheckpointTip,
    ) -> Self {
        Self {
            sequencer_predicate,
            checkpoint_predicate,
            verified_tip,
            available_deposit_sum: 0,
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

    /// Returns the available deposit sum in satoshis.
    pub fn available_deposit_sum(&self) -> u64 {
        self.available_deposit_sum
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

    /// Records a processed deposit, incrementing the available deposit sum.
    pub(crate) fn record_deposit(&mut self, amount: BitcoinAmount) {
        self.available_deposit_sum += amount.to_sat();
    }

    /// Checks whether the available deposit sum can cover all withdrawal intents.
    pub(crate) fn can_honor_withdrawals(&self, withdrawal_intents: &[WithdrawOutput]) -> bool {
        let total_withdrawal: u64 = withdrawal_intents.iter().map(|w| w.amt().to_sat()).sum();
        self.available_deposit_sum >= total_withdrawal
    }

    /// Deducts the total withdrawal amount from the available deposit sum.
    ///
    /// # Panics
    ///
    /// Panics if the total withdrawal exceeds the available deposit sum. Callers must verify
    /// with [`can_honor_withdrawals`](Self::can_honor_withdrawals) first.
    pub(crate) fn deduct_withdrawals(&mut self, withdrawal_intents: &[WithdrawOutput]) {
        let total_withdrawal: u64 = withdrawal_intents.iter().map(|w| w.amt().to_sat()).sum();
        self.available_deposit_sum = self
            .available_deposit_sum
            .checked_sub(total_withdrawal)
            .expect("deduct_withdrawals called without sufficient deposit backing");
    }
}
