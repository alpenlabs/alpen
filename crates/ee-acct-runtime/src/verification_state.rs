//! Verification state accumulator.

use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::{EeAccountState, EnvError, EnvResult};
use strata_ee_chain_types::BlockOutputs;
use strata_snark_acct_types::{OutputMessage, OutputTransfer, UpdateOutputs};

type Hash = [u8; 32];

/// State tracker that accumulates changes that we need to make checks about
/// later on in update processing.
#[derive(Debug)]
pub(crate) struct UpdateVerificationState {
    // balance bookkeeping as additional checks to avoid overdraw
    #[expect(dead_code, reason = "for future use")]
    orig_tracked_balance: BitcoinAmount,
    total_val_sent: BitcoinAmount,
    #[expect(dead_code, reason = "for future use")]
    total_val_recv: BitcoinAmount,

    // commits to check
    pending_commits: Vec<PendingCommit>,

    // number of inputs we've consumed
    consumed_inputs: usize,

    // recorded outputs we'll check later
    accumulated_outputs: UpdateOutputs,

    // Recorded DA.
    #[expect(dead_code, reason = "for future use")]
    l1_da_blob_hashes: Vec<Hash>,
}

impl UpdateVerificationState {
    /// Constructs a verification state using the account's initial state as a
    /// reference.
    ///
    /// We don't take ownership of it, because that makes the types less clean
    /// to work with later on and breaks our use of the type system to enforce
    /// correctness about not updating the state with private information.
    pub(crate) fn new_from_state(state: &EeAccountState) -> Self {
        Self {
            orig_tracked_balance: state.tracked_balance(),
            total_val_sent: 0.into(),
            total_val_recv: 0.into(),
            pending_commits: Vec::new(),
            consumed_inputs: 0,
            accumulated_outputs: UpdateOutputs::new_empty(),
            l1_da_blob_hashes: Vec::new(),
        }
    }

    pub(crate) fn pending_commits(&self) -> &[PendingCommit] {
        &self.pending_commits
    }

    pub(crate) fn add_pending_commit(&mut self, commit: PendingCommit) {
        self.pending_commits.push(commit);
    }

    #[expect(dead_code, reason = "for future use")]
    pub(crate) fn consumed_inputs(&self) -> usize {
        self.consumed_inputs
    }

    /// Increments the number of consumed inputs by some amount.
    pub(crate) fn inc_consumed_inputs(&mut self, amt: usize) {
        self.consumed_inputs += amt;
    }

    #[expect(dead_code, reason = "for future use")]
    pub(crate) fn accumulated_outputs(&self) -> &UpdateOutputs {
        &self.accumulated_outputs
    }

    /// Appends a package block's outputs into the pending outputs being
    /// built internally.  This way we can compare it against the update op data
    /// later.
    pub(crate) fn merge_block_outputs(&mut self, outputs: &BlockOutputs) {
        // Just merge the entries into the buffer.  This is a little more
        // complicated than it really is because we have to convert between two
        // sets of similar types that are separately defined to avoid semantic
        // confusion because they do refer to different concepts.
        self.accumulated_outputs.transfers_mut().extend(
            outputs
                .output_transfers()
                .iter()
                .map(|e| OutputTransfer::new(e.dest(), e.value())),
        );

        self.accumulated_outputs.messages_mut().extend(
            outputs
                .output_messages()
                .iter()
                .map(|e| OutputMessage::new(e.dest(), e.payload().clone())),
        );

        // Annoying thing to do checked summation.
        let sent_amts_iter = [self.total_val_sent]
            .into_iter()
            .chain(outputs.output_transfers().iter().map(|e| e.value()))
            .chain(
                outputs
                    .output_messages()
                    .iter()
                    .map(|e| e.payload().value()),
            );

        self.total_val_sent = BitcoinAmount::sum(sent_amts_iter);
    }

    /// Final checks to see if there's anything in the verification state that
    /// were supposed to have been dealt with but weren't.
    pub(crate) fn check_obligations(&self) -> EnvResult<()> {
        // TODO
        Ok(())
    }
}

/// Data about a pending commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingCommit {
    new_tip_exec_blkid: Hash,
}

impl PendingCommit {
    pub(crate) fn new(new_tip_exec_blkid: Hash) -> Self {
        Self { new_tip_exec_blkid }
    }

    pub(crate) fn new_tip_exec_blkid(&self) -> [u8; 32] {
        self.new_tip_exec_blkid
    }
}

/// Tracks a list of entries that we want to compare a sequence of them against.
///
/// We use this for processing pending inputs while processing notpackages.
pub(crate) struct InputTracker<'a, T> {
    expected_inputs: &'a [T],
    consumed: usize,
}

impl<'a, T> InputTracker<'a, T> {
    pub(crate) fn new(expected_inputs: &'a [T]) -> Self {
        Self {
            expected_inputs,
            consumed: 0,
        }
    }

    /// Gets the number of entries consumed.
    pub(crate) fn consumed(&self) -> usize {
        self.consumed
    }

    /// Gets if there are more entries that could be consumed.
    #[cfg(test)]
    fn has_next(&self) -> bool {
        self.consumed < self.expected_inputs.len()
    }

    /// Gets the next entry that would need to be be consumed, if there is one.
    fn expected_next(&self) -> Option<&'a T> {
        self.expected_inputs.get(self.consumed)
    }
}

impl<'a, T: Eq + PartialEq> InputTracker<'a, T> {
    /// Checks if an input matches the next value we expect to consume.  If it
    /// matches, increments the pointer.  Errors on mismatch.
    pub(crate) fn consume_input(&mut self, input: &T) -> EnvResult<()> {
        let Some(exp_next) = self.expected_next() else {
            return Err(EnvError::MalformedCoinput);
        };

        if input != exp_next {
            return Err(EnvError::MalformedCoinput);
        }

        self.consumed += 1;
        Ok(())
    }

    /// Gets the remaining unconsumed entries.
    pub(crate) fn remaining(&self) -> &'a [T] {
        &self.expected_inputs[self.consumed..]
    }

    /// Checks if all entries have been consumed.
    pub(crate) fn is_empty(&self) -> bool {
        self.consumed >= self.expected_inputs.len()
    }

    /// Advances the tracker by `count` entries without checking them.
    ///
    /// This should only be called after validation has been performed.
    pub(crate) fn advance_unchecked(&mut self, count: usize) {
        self.consumed += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_tracker_new() {
        let inputs = vec![1, 2, 3];
        let tracker = InputTracker::new(&inputs);

        assert_eq!(tracker.consumed(), 0);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&1));
    }

    #[test]
    fn test_input_tracker_empty() {
        let inputs: Vec<i32> = vec![];
        let tracker = InputTracker::new(&inputs);

        assert_eq!(tracker.consumed(), 0);
        assert!(!tracker.has_next());
        assert_eq!(tracker.expected_next(), None);
    }

    #[test]
    fn test_input_tracker_consume_matching() {
        let inputs = vec![1, 2, 3];
        let mut tracker = InputTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&2));

        assert!(tracker.consume_input(&2).is_ok());
        assert_eq!(tracker.consumed(), 2);
        assert!(tracker.has_next());
        assert_eq!(tracker.expected_next(), Some(&3));

        assert!(tracker.consume_input(&3).is_ok());
        assert_eq!(tracker.consumed(), 3);
        assert!(!tracker.has_next());
        assert_eq!(tracker.expected_next(), None);
    }

    #[test]
    fn test_input_tracker_consume_mismatch() {
        let inputs = vec![1, 2, 3];
        let mut tracker = InputTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);

        let result = tracker.consume_input(&99);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EnvError::MalformedCoinput));
        assert_eq!(tracker.consumed(), 1); // consumed count unchanged on error
    }

    #[test]
    fn test_input_tracker_consume_beyond_end() {
        let inputs = vec![1];
        let mut tracker = InputTracker::new(&inputs);

        assert!(tracker.consume_input(&1).is_ok());
        assert_eq!(tracker.consumed(), 1);
        assert!(!tracker.has_next());

        let result = tracker.consume_input(&2);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EnvError::MalformedCoinput));
        assert_eq!(tracker.consumed(), 1);
    }

    #[test]
    fn test_input_tracker_consume_wrong_order() {
        let inputs = vec![1, 2, 3];
        let mut tracker = InputTracker::new(&inputs);

        let result = tracker.consume_input(&2);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EnvError::MalformedCoinput));
        assert_eq!(tracker.consumed(), 0);
    }

    #[test]
    fn test_input_tracker_string_type() {
        let inputs = vec!["foo".to_string(), "bar".to_string(), "baz".to_string()];
        let mut tracker = InputTracker::new(&inputs);

        assert!(tracker.consume_input(&"foo".to_string()).is_ok());
        assert_eq!(tracker.consumed(), 1);

        assert!(tracker.consume_input(&"bar".to_string()).is_ok());
        assert_eq!(tracker.consumed(), 2);

        let result = tracker.consume_input(&"wrong".to_string());
        assert!(result.is_err());
        assert_eq!(tracker.consumed(), 2);
    }
}
