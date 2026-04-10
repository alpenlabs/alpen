use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_bridge_msgs::WithdrawOutput;
use strata_asm_params::CheckpointInitConfig;
use strata_btc_types::BitcoinAmount;
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_identifiers::L2BlockCommitment;
use strata_predicate::PredicateKey;

use crate::errors::InvalidCheckpointPayload;

/// Opaque proof token for a verified set of withdrawal intents.
///
/// Produced by the checkpoint state's withdrawal verification and consumed by its deduction
/// method, enforcing at the type level that the fund deduction can only happen after successful
/// denomination-level verification.
///
/// This type has no public constructor or accessors, and is neither [`Clone`] nor [`Copy`],
/// so that each verification produces exactly one deduction.
#[derive(Debug)]
pub struct VerifiedWithdrawals(BTreeMap<BitcoinAmount, u32>);

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

    /// Available bridge UTXOs tracked by denomination.
    ///
    /// Maps each deposit denomination to the count of UTXOs at that denomination that have
    /// been processed by the bridge but not yet consumed by withdrawal dispatches. Used for
    /// rejecting checkpoints whose withdrawal intents cannot be matched to available UTXOs.
    available_funds: BTreeMap<BitcoinAmount, u32>,
}

impl CheckpointState {
    /// Initializes checkpoint state from configuration.
    pub fn init(config: CheckpointInitConfig) -> Self {
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
            available_funds: BTreeMap::new(),
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

    /// Returns the total available deposit value in satoshis, derived from the
    /// denomination-keyed fund map.
    pub fn available_deposit_sum(&self) -> u64 {
        self.available_funds
            .iter()
            .map(|(denom, count)| denom.to_sat() * (*count as u64))
            .sum()
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

    /// Records a processed deposit, incrementing the UTXO count for this denomination.
    pub(crate) fn record_deposit(&mut self, amount: BitcoinAmount) {
        *self.available_funds.entry(amount).or_insert(0) += 1;
    }

    /// Verifies that the available funds can cover all withdrawal intents.
    ///
    /// Does not mutate state. On success returns a [`VerifiedWithdrawals`] token that must
    /// be passed to [`deduct_withdrawals`](Self::deduct_withdrawals) to apply the deduction.
    /// This enforces at the type level deduction can only happen after successful verification.
    ///
    /// Each intent's amount is greedily decomposed into available UTXO denominations
    /// (largest first). This supports both single-denomination and batch withdrawal intents.
    pub(crate) fn verify_can_honor_withdrawals(
        &self,
        withdrawal_intents: &[WithdrawOutput],
    ) -> Result<VerifiedWithdrawals, InvalidCheckpointPayload> {
        let mut funds = self.available_funds.clone();

        let insufficient = || InvalidCheckpointPayload::InsufficientFunds {
            available_sat: self.available_deposit_sum(),
            required_sat: withdrawal_intents.iter().map(|w| w.amt().to_sat()).sum(),
        };

        for intent in withdrawal_intents {
            funds = get_funds_after_withdrawal(&funds, intent.amt()).ok_or_else(&insufficient)?;
        }

        Ok(VerifiedWithdrawals(funds))
    }

    /// Applies the pre-verified withdrawal deduction to state.
    ///
    /// Requires a [`VerifiedWithdrawals`] token, which can only be obtained from
    /// [`verify_can_honor_withdrawals`](Self::verify_can_honor_withdrawals).
    pub(crate) fn deduct_withdrawals(&mut self, token: VerifiedWithdrawals) {
        self.available_funds = token.0;
    }
}

/// Computes the remaining funds after greedily deducting UTXOs to cover a withdrawal amount.
///
/// Consumes largest denominations first. Returns `Some(remaining_funds)` if the amount
/// is exactly covered, `None` if it cannot be covered. Does not modify the input.
///
/// Note: this function may not play well in the multi-denomination setting and user-assigned
/// withdrawals, because it prioritises the amount over the assigned operator.
fn get_funds_after_withdrawal(
    funds: &BTreeMap<BitcoinAmount, u32>,
    amount: BitcoinAmount,
) -> Option<BTreeMap<BitcoinAmount, u32>> {
    let mut result = funds.clone();
    let mut remaining = amount.to_sat();

    for (denom, count) in result.iter_mut().rev() {
        if remaining == 0 {
            break;
        }
        let denom_sat = denom.to_sat();
        if denom_sat > remaining {
            continue;
        }

        let n = (remaining / denom_sat).min(*count as u64) as u32;
        *count -= n;
        remaining -= n as u64 * denom_sat;
    }

    (remaining == 0).then(|| {
        result.retain(|_, c| *c > 0);
        result
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bitcoin_bosd::Descriptor;
    use strata_asm_bridge_msgs::WithdrawOutput;
    use strata_btc_types::BitcoinAmount;
    use strata_checkpoint_types_ssz::CheckpointTip;
    use strata_identifiers::L2BlockCommitment;
    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::{CheckpointState, get_funds_after_withdrawal};
    use crate::errors::InvalidCheckpointPayload;

    fn funds(entries: &[(u64, u32)]) -> BTreeMap<BitcoinAmount, u32> {
        entries
            .iter()
            .map(|(sats, count)| (BitcoinAmount::from_sat(*sats), *count))
            .collect()
    }

    fn sat(sats: u64) -> BitcoinAmount {
        BitcoinAmount::from_sat(sats)
    }

    // --- get_funds_after_withdrawal tests ---

    #[test]
    fn test_greedy_single_denom_exact() {
        let f = funds(&[(100_000_000, 5)]);
        let result = get_funds_after_withdrawal(&f, sat(100_000_000)).unwrap();
        assert_eq!(result, funds(&[(100_000_000, 4)]));
    }

    #[test]
    fn test_greedy_single_denom_batch() {
        let f = funds(&[(100_000_000, 5)]);
        let result = get_funds_after_withdrawal(&f, sat(300_000_000)).unwrap();
        assert_eq!(result, funds(&[(100_000_000, 2)]));
    }

    #[test]
    fn test_greedy_single_denom_insufficient() {
        let f = funds(&[(100_000_000, 2)]);
        assert!(get_funds_after_withdrawal(&f, sat(300_000_000)).is_none());
    }

    #[test]
    fn test_greedy_non_multiple() {
        let f = funds(&[(100_000_000, 5)]);
        assert!(get_funds_after_withdrawal(&f, sat(150_000_000)).is_none());
    }

    #[test]
    fn test_greedy_withdrawal_below_denom() {
        // 0.5 BTC requested but all UTXOs are 1 BTC or larger — nothing usable
        let f = funds(&[(100_000_000, 3), (200_000_000, 2)]);
        assert!(get_funds_after_withdrawal(&f, sat(50_000_000)).is_none());
    }

    #[test]
    fn test_greedy_zero_amount() {
        let f = funds(&[(100_000_000, 5)]);
        let result = get_funds_after_withdrawal(&f, sat(0)).unwrap();
        assert_eq!(result, f);
    }

    #[test]
    fn test_greedy_empty_funds() {
        let f = funds(&[]);
        assert!(get_funds_after_withdrawal(&f, sat(100_000_000)).is_none());
    }

    #[test]
    fn test_greedy_multi_denom_largest_first() {
        // 2 BTC and 5 BTC available, withdraw 7 BTC
        let f = funds(&[(200_000_000, 1), (500_000_000, 1)]);
        let result = get_funds_after_withdrawal(&f, sat(700_000_000)).unwrap();
        assert_eq!(result, funds(&[]));
    }

    #[test]
    fn test_greedy_multi_denom_partial() {
        // 1 BTC x3 and 2 BTC x2, withdraw 5 BTC = 2x2 BTC + 1x1 BTC
        let f = funds(&[(100_000_000, 3), (200_000_000, 2)]);
        let result = get_funds_after_withdrawal(&f, sat(500_000_000)).unwrap();
        assert_eq!(result, funds(&[(100_000_000, 2)]));
    }

    #[test]
    fn test_greedy_skips_high_denom_uses_lower() {
        // 1 BTC x3 and 5 BTC x1 available, withdraw 2 BTC:
        // greedy skips 5 BTC (too large), uses two 1 BTC UTXOs
        let f = funds(&[(100_000_000, 3), (500_000_000, 1)]);
        let result = get_funds_after_withdrawal(&f, sat(200_000_000)).unwrap();
        assert_eq!(result, funds(&[(100_000_000, 1), (500_000_000, 1)]));
    }

    #[test]
    fn test_greedy_does_not_modify_input() {
        let f = funds(&[(100_000_000, 5)]);
        let _ = get_funds_after_withdrawal(&f, sat(300_000_000));
        assert_eq!(f, funds(&[(100_000_000, 5)]));
    }

    fn dummy_state() -> CheckpointState {
        let tip = CheckpointTip::new(0, 100, L2BlockCommitment::null());
        let predicate = PredicateKey::new(PredicateTypeId::AlwaysAccept, vec![]);
        CheckpointState::new(predicate.clone(), predicate, tip)
    }

    fn dummy_descriptor() -> Descriptor {
        Descriptor::new_p2wpkh(&[0u8; 20])
    }

    fn withdrawal(sats: u64) -> WithdrawOutput {
        WithdrawOutput::new(dummy_descriptor(), BitcoinAmount::from_sat(sats))
    }

    #[test]
    fn test_record_deposit_tracks_by_denomination() {
        let mut state = dummy_state();
        let denom_5btc = BitcoinAmount::from_sat(500_000_000);
        let denom_10btc = BitcoinAmount::from_sat(1_000_000_000);

        state.record_deposit(denom_5btc);
        state.record_deposit(denom_5btc);
        state.record_deposit(denom_10btc);

        assert_eq!(state.available_deposit_sum(), 2_000_000_000);
    }

    #[test]
    fn test_deduct_exact_denomination_match() {
        let mut state = dummy_state();
        let denom = BitcoinAmount::from_sat(500_000_000);

        state.record_deposit(denom);
        state.record_deposit(denom);

        let intents = vec![withdrawal(500_000_000)];
        let token = state.verify_can_honor_withdrawals(&intents).unwrap();
        state.deduct_withdrawals(token);
        assert_eq!(state.available_deposit_sum(), 500_000_000);
    }

    #[test]
    fn test_denomination_mismatch_fails() {
        let mut state = dummy_state();
        // One 10 BTC deposit
        state.record_deposit(BitcoinAmount::from_sat(1_000_000_000));

        // Two 5 BTC withdrawals — total (10 BTC) matches, but no 5 BTC UTXOs exist
        let intents = vec![withdrawal(500_000_000), withdrawal(500_000_000)];
        let err = state.verify_can_honor_withdrawals(&intents).unwrap_err();

        assert!(matches!(
            err,
            InvalidCheckpointPayload::InsufficientFunds {
                available_sat: 1_000_000_000,
                required_sat: 1_000_000_000,
            }
        ));

        // State should be unchanged
        assert_eq!(state.available_deposit_sum(), 1_000_000_000);
    }

    #[test]
    fn test_non_divisible_withdrawal_fails() {
        let mut state = dummy_state();
        state.record_deposit(BitcoinAmount::from_sat(100_000_000));
        state.record_deposit(BitcoinAmount::from_sat(100_000_000));

        // 1.5 BTC cannot be covered by 1 BTC denominations
        let intents = vec![withdrawal(150_000_000)];
        assert!(state.verify_can_honor_withdrawals(&intents).is_err());
        assert_eq!(state.available_deposit_sum(), 200_000_000);
    }

    #[test]
    fn test_insufficient_count_fails() {
        let mut state = dummy_state();
        let denom = BitcoinAmount::from_sat(500_000_000);

        state.record_deposit(denom); // Only 1 UTXO

        // Try to withdraw 2 UTXOs of same denomination
        let intents = vec![withdrawal(500_000_000), withdrawal(500_000_000)];
        assert!(state.verify_can_honor_withdrawals(&intents).is_err());

        // State unchanged
        assert_eq!(state.available_deposit_sum(), 500_000_000);
    }

    #[test]
    fn test_batch_withdrawal_single_denom() {
        let mut state = dummy_state();
        let denom = BitcoinAmount::from_sat(100_000_000);

        // 5 deposits of 1 BTC each
        for _ in 0..5 {
            state.record_deposit(denom);
        }

        // Batch withdrawal of 3 BTC (single intent)
        let intents = vec![withdrawal(300_000_000)];
        let token = state.verify_can_honor_withdrawals(&intents).unwrap();
        state.deduct_withdrawals(token);
        assert_eq!(state.available_deposit_sum(), 200_000_000);
    }

    #[test]
    fn test_empty_intents_succeeds() {
        let mut state = dummy_state();
        state.record_deposit(BitcoinAmount::from_sat(500_000_000));

        assert!(state.verify_can_honor_withdrawals(&[]).is_ok());
        assert_eq!(state.available_deposit_sum(), 500_000_000);
    }
}
