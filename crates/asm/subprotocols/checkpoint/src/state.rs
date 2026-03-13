use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
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
pub struct VerifiedWithdrawals(AvailableFunds);

/// An entry in the available funds.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEncode, DeriveDecode)]
struct AvailableFundsEntry {
    /// The denomination of the available funds.
    denomination: BitcoinAmount,

    /// The count of the available funds.
    count: u32,
}

/// Available funds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AvailableFunds {
    /// The entries of the available funds.
    entries: Vec<AvailableFundsEntry>,
}

impl AvailableFunds {
    /// Creates a new empty available funds.
    fn new() -> Self {
        Self::default()
    }

    /// Converts a vector of available funds entries to an available funds.
    fn from_entries(entries: Vec<AvailableFundsEntry>) -> Result<Self, DecodeError> {
        for entry in &entries {
            if entry.count == 0 {
                return Err(DecodeError::BytesInvalid(
                    "available funds count cannot be zero".into(),
                ));
            }
        }

        if entries
            .windows(2)
            .any(|pair| pair[0].denomination >= pair[1].denomination)
        {
            return Err(DecodeError::BytesInvalid(
                "available funds entries must be strictly sorted by denomination".into(),
            ));
        }

        Ok(Self { entries })
    }

    /// Returns an iterator over the available funds entries.
    fn iter(&self) -> impl Iterator<Item = (&BitcoinAmount, &u32)> {
        self.entries
            .iter()
            .map(|entry| (&entry.denomination, &entry.count))
    }

    /// Increments the count of the available funds for the given denomination.
    fn increment(&mut self, denomination: BitcoinAmount) {
        match self
            .entries
            .binary_search_by_key(&denomination, |entry| entry.denomination)
        {
            Ok(index) => self.entries[index].count += 1,
            Err(index) => self.entries.insert(
                index,
                AvailableFundsEntry {
                    denomination,
                    count: 1,
                },
            ),
        }
    }

    /// Decrements the count of the available funds for the given denomination.
    fn decrement(&mut self, denomination: BitcoinAmount) -> bool {
        let Ok(index) = self
            .entries
            .binary_search_by_key(&denomination, |entry| entry.denomination)
        else {
            return false;
        };

        let Some(next_count) = self.entries[index].count.checked_sub(1) else {
            return false;
        };

        if next_count == 0 {
            self.entries.remove(index);
        } else {
            self.entries[index].count = next_count;
        }

        true
    }
}

/// SSZ-friendly representation of [`AvailableFunds`].
#[derive(DeriveEncode, DeriveDecode)]
struct AvailableFundsSsz {
    /// The entries of the available funds.
    entries: Vec<AvailableFundsEntry>,
}

impl Encode for AvailableFunds {
    fn is_ssz_fixed_len() -> bool {
        <AvailableFundsSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AvailableFundsSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        AvailableFundsSsz {
            entries: self.entries.clone(),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        AvailableFundsSsz {
            entries: self.entries.clone(),
        }
        .ssz_bytes_len()
    }
}

impl Decode for AvailableFunds {
    fn is_ssz_fixed_len() -> bool {
        <AvailableFundsSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AvailableFundsSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = AvailableFundsSsz::from_ssz_bytes(bytes)?;
        Self::from_entries(value.entries)
    }
}

/// Checkpoint subprotocol state.
#[derive(Clone, Debug, PartialEq, DeriveEncode, DeriveDecode)]
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
    available_funds: AvailableFunds,
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
            available_funds: AvailableFunds::new(),
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
        self.available_funds.increment(amount);
    }

    /// Verifies that the available funds can cover all withdrawal intents using **exact
    /// denomination matching**.
    ///
    /// Does not mutate state. On success returns a [`VerifiedWithdrawals`] token that must
    /// be passed to [`deduct_withdrawals`](Self::deduct_withdrawals) to apply the deduction.
    /// This enforces at the type level deduction can only happen after successful verification.
    ///
    /// N.B. Despite bridge targeting single denomination deposits, the checkpoint withdrawals
    /// verification logic below allows for multi denominations to be successfully handled (as
    /// long as they were successfully processed by the bridge according to the ASM protocol).
    /// The algorithm is quite naive. For instance, if bridge somehow was able to process
    /// the deposits of 2 BTC and 5 BTC, then that means checkpoints with withdrawal intents
    /// of 2 and 5 BTC are valid (as long as they can be honored by its count), but 7(=2+5) BTC
    /// is not supported (and such checkpoints would be treated as invalid and thus skipped).
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
            let denom = intent.amt();
            if !funds.decrement(denom) {
                return Err(insufficient());
            }
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

#[cfg(test)]
mod tests {
    use bitcoin_bosd::Descriptor;
    use ssz::{Decode, Encode};
    use strata_asm_bridge_msgs::WithdrawOutput;
    use strata_btc_types::BitcoinAmount;
    use strata_checkpoint_types_ssz::CheckpointTip;
    use strata_identifiers::L2BlockCommitment;
    use strata_predicate::{PredicateKey, PredicateTypeId};

    use super::CheckpointState;
    use crate::errors::InvalidCheckpointPayload;

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
    fn test_empty_intents_succeeds() {
        let mut state = dummy_state();
        state.record_deposit(BitcoinAmount::from_sat(500_000_000));

        assert!(state.verify_can_honor_withdrawals(&[]).is_ok());
        assert_eq!(state.available_deposit_sum(), 500_000_000);
    }

    #[test]
    fn test_checkpoint_state_ssz_roundtrip() {
        let mut state = dummy_state();
        state.record_deposit(BitcoinAmount::from_sat(500_000_000));
        state.record_deposit(BitcoinAmount::from_sat(1_000_000_000));

        let encoded = state.as_ssz_bytes();
        let decoded = CheckpointState::from_ssz_bytes(&encoded).expect("ssz decode should succeed");

        assert_eq!(state, decoded);
    }
}
