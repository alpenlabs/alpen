//! Verification state accumulator.

use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::{EeAccountState, EnvError, EnvResult, PendingInputEntry};
use strata_ee_chain_types::BlockOutputs;
use strata_snark_acct_types::{OutputMessage, OutputTransfer, UpdateOutputs};

type Hash = [u8; 32];

/// State tracker that accumulates changes that we need to make checks about
/// later on in update processing.
#[derive(Debug)]
pub struct UpdateVerificationState {
    // balance bookkeeping as additional checks to avoid overdraw
    orig_tracked_balance: BitcoinAmount,
    total_val_sent: BitcoinAmount,
    total_val_recv: BitcoinAmount,

    // commits to check
    pending_commits: Vec<PendingCommit>,

    // number of inputs we've consumed
    consumed_inputs: usize,

    // recorded outputs we'll check later
    accumulated_outputs: UpdateOutputs,

    // Recorded DA.
    l1_da_blob_hashes: Vec<Hash>,
}

impl UpdateVerificationState {
    /// Constructs a verification state using the account's initial state as a
    /// reference.
    ///
    /// We don't take ownership of it, because that makes the types less clean
    /// to work with later on and breaks our use of the type system to enforce
    /// correctness about not updating the state with private information.
    pub fn new_from_state(state: &EeAccountState) -> Self {
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

    pub fn pending_commits(&self) -> &[PendingCommit] {
        &self.pending_commits
    }

    pub fn add_pending_commit(&mut self, commit: PendingCommit) {
        self.pending_commits.push(commit);
    }

    pub fn consumed_inputs(&self) -> usize {
        self.consumed_inputs
    }

    /// Increments the number of consumed inputs by some amount.
    pub fn inc_consumed_inputs(&mut self, amt: usize) {
        self.consumed_inputs += amt;
    }

    pub fn accumulated_outputs(&self) -> &UpdateOutputs {
        &self.accumulated_outputs
    }

    /// Appends a notpackage block's outputs into the pending outputs being
    /// built internally.  This way we can compare it against the update op data
    /// later.
    pub fn merge_block_outputs(&mut self, outputs: &BlockOutputs) {
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

/// Tracks pending inputs for while processing notpackages.
pub(crate) struct InputTracker<'a> {
    expected_inputs: &'a [PendingInputEntry],
    consumed: usize,
}

impl<'a> InputTracker<'a> {
    pub(crate) fn new(expected_inputs: &'a [PendingInputEntry]) -> Self {
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
    fn has_next(&self) -> bool {
        self.consumed < self.expected_inputs.len()
    }

    /// Gets the next entry that would need to be be consumed, if there is one.
    fn expected_next(&self) -> Option<&'a PendingInputEntry> {
        self.expected_inputs.get(self.consumed)
    }

    /// Checks if an input matches the next value we expect to consume.  If it
    /// matches, increments the pointer.  Errors on mismatch.
    pub(crate) fn consume_input(&mut self, input: &PendingInputEntry) -> EnvResult<()> {
        let Some(exp_next) = self.expected_next() else {
            return Err(EnvError::MalformedCoinput);
        };

        if input != exp_next {
            return Err(EnvError::MalformedCoinput);
        }

        Ok(())
    }
}
