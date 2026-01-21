//! Verification state accumulator.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_ee_acct_types::{EeAccountState, EnvResult};
use strata_ee_chain_types::ExecOutputs;
use strata_snark_acct_types::{
    MAX_MESSAGES, MAX_TRANSFERS, OutputMessage, OutputTransfer, UpdateOutputs,
};

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
    pub(crate) fn merge_block_outputs(&mut self, outputs: &ExecOutputs) {
        // Just merge the entries into the buffer.  This is a little more
        // complicated than it really is because we have to convert between two
        // sets of similar types that are separately defined to avoid semantic
        // confusion because they do refer to different concepts.
        //
        // This panic should never happen: capacity limit is [`MAX_TRANSFERS`].
        // If hit, it indicates either a malicious block or a bug. We panic to
        // fail fast rather than continue with inconsistent state.
        self.accumulated_outputs
            .try_extend_transfers(
                outputs
                    .output_transfers()
                    .iter()
                    .map(|e| OutputTransfer::new(e.dest(), e.value())),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "output transfers capacity exceeded: {e}. Current: {}, Adding: {}, Max: {}",
                    self.accumulated_outputs.transfers().len(),
                    outputs.output_transfers().len(),
                    MAX_TRANSFERS
                )
            });

        // This panic should never happen: capacity limit is [`MAX_TRANSFERS`].
        // If hit, it indicates either a malicious block or a bug. We panic to
        // fail fast rather than continue with inconsistent state.
        self.accumulated_outputs
            .try_extend_messages(
                outputs
                    .output_messages()
                    .iter()
                    .map(|e| OutputMessage::new(e.dest(), e.payload().clone())),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "output messages capacity exceeded: {e}. Current: {}, Adding: {}, Max: {}",
                    self.accumulated_outputs.messages().len(),
                    outputs.output_messages().len(),
                    MAX_MESSAGES
                )
            });

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

    pub(crate) fn new_tip_exec_blkid(&self) -> Hash {
        self.new_tip_exec_blkid
    }
}
