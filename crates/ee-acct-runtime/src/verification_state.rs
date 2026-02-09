//! Verification state for EE accounts.
//!
//! This module contains the verification state types used during update
//! processing in SNARK proofs.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState, EnvResult, ExecutionEnvironment};
use strata_ee_chain_types::ExecOutputs;
use strata_snark_acct_types::{
    MAX_MESSAGES, MAX_TRANSFERS, OutputMessage, OutputTransfer, UpdateOutputs,
};

use crate::{commit::PendingCommit, private_input::SharedPrivateInput};

/// Verification input for EE accounts.
///
/// Contains references to:
/// - The shared private input (chain segments, prev header, pre-state)
/// - The execution environment for block execution
///
/// This is passed by value to `start_verification` when using the verification
/// path, so that its contents (the references) can be moved into `VState`.
#[expect(missing_debug_implementations, reason = "E may not implement Debug")]
pub struct EeVerificationInput<'a, E: ExecutionEnvironment> {
    /// Shared private input data.
    pub shared_private: &'a SharedPrivateInput,

    /// Execution environment for block execution.
    pub ee: &'a E,
}

impl<'a, E: ExecutionEnvironment> EeVerificationInput<'a, E> {
    /// Creates new verification input.
    pub fn new(shared_private: &'a SharedPrivateInput, ee: &'a E) -> Self {
        Self { shared_private, ee }
    }
}

/// Verification state for EE accounts.
///
/// This type tracks all verification-related state during update processing,
/// including balance bookkeeping, pending commits, outputs, and references to
/// the private input data needed for chain segment verification.
#[expect(missing_debug_implementations, reason = "E may not implement Debug")]
pub struct EeVerificationState<'a, E: ExecutionEnvironment> {
    // Balance bookkeeping as additional checks to avoid overdraw
    #[expect(dead_code, reason = "for future use")]
    orig_tracked_balance: BitcoinAmount,

    total_val_sent: BitcoinAmount,

    #[expect(dead_code, reason = "for future use")]
    total_val_recv: BitcoinAmount,

    // Commits to check
    pending_commits: Vec<PendingCommit>,

    // Number of inputs we've consumed
    consumed_inputs: usize,

    // Recorded outputs we'll check later
    accumulated_outputs: UpdateOutputs,

    // Recorded DA.
    #[expect(dead_code, reason = "for future use")]
    l1_da_blob_hashes: Vec<Hash>,

    /// Execution environment for block execution.
    ee: &'a E,

    /// Chain segments to verify.
    commit_data: &'a [CommitChainSegment],

    /// Previous header that we already have in our state.
    raw_prev_header: &'a [u8],

    /// Partial pre-state corresponding to the previous header.
    raw_partial_pre_state: &'a [u8],
}

impl<'a, E: ExecutionEnvironment> EeVerificationState<'a, E> {
    /// Constructs a verification state using the account's initial state as a
    /// reference, along with the verification input data.
    pub fn new_from_state(
        state: &EeAccountState,
        ee: &'a E,
        commit_data: &'a [CommitChainSegment],
        raw_prev_header: &'a [u8],
        raw_partial_pre_state: &'a [u8],
    ) -> Self {
        Self {
            orig_tracked_balance: state.tracked_balance(),
            total_val_sent: 0.into(),
            total_val_recv: 0.into(),
            pending_commits: Vec::new(),
            consumed_inputs: 0,
            accumulated_outputs: UpdateOutputs::new_empty(),
            l1_da_blob_hashes: Vec::new(),
            ee,
            commit_data,
            raw_prev_header,
            raw_partial_pre_state,
        }
    }

    /// Returns the pending commits.
    pub(crate) fn pending_commits(&self) -> &[PendingCommit] {
        &self.pending_commits
    }

    /// Adds a pending commit.
    pub(crate) fn add_pending_commit(&mut self, commit: PendingCommit) {
        self.pending_commits.push(commit);
    }

    /// Returns the number of consumed inputs.
    #[expect(dead_code, reason = "for future use")]
    pub(crate) fn consumed_inputs(&self) -> usize {
        self.consumed_inputs
    }

    /// Increments the number of consumed inputs by some amount.
    pub(crate) fn inc_consumed_inputs(&mut self, amt: usize) {
        self.consumed_inputs += amt;
    }

    /// Returns the accumulated outputs.
    #[expect(dead_code, reason = "for future use")]
    pub(crate) fn accumulated_outputs(&self) -> &UpdateOutputs {
        &self.accumulated_outputs
    }

    /// Appends a package block's outputs into the pending outputs being
    /// built internally. This way we can compare it against the update op data
    /// later.
    pub(crate) fn merge_block_outputs(&mut self, outputs: &ExecOutputs) {
        // Just merge the entries into the buffer. This is a little more
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

        // This panic should never happen: capacity limit is [`MAX_MESSAGES`].
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
        // TODO check these obligations!  this is all being refactored for
        // chunk/multiproof updates soon so this is fine to be a no-op
        Ok(())
    }

    /// Returns the chain segments to verify.
    pub fn commit_data(&self) -> &'a [CommitChainSegment] {
        self.commit_data
    }

    /// Returns the raw previous header.
    pub fn raw_prev_header(&self) -> &'a [u8] {
        self.raw_prev_header
    }

    /// Returns the raw partial pre-state.
    pub fn raw_partial_pre_state(&self) -> &'a [u8] {
        self.raw_partial_pre_state
    }

    /// Returns the execution environment.
    pub fn ee(&self) -> &'a E {
        self.ee
    }
}
