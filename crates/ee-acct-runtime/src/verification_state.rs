//! Verification state for EE accounts.
//!
//! This module contains the verification state types used during update
//! processing in SNARK proofs.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_ee_acct_types::{
    EeAccountState, EnvError, EnvProgramResult, EnvResult, ExecutionEnvironment, PendingInputEntry,
    UpdateExtraData,
};
use strata_ee_chain_types::{ChunkTransition, ExecOutputs, SequenceTracker};
use strata_predicate::{PredicateKey, PredicateKeyBuf};
use strata_snark_acct_types::{OutputMessage, OutputTransfer, UpdateOutputs};

use crate::private_input::ArchivedChunkInput;

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
    /// Execution environment for block execution.
    ee: &'a E,

    /// Predicate used for verifying chunk proofs.
    chunk_predicate_key: &'a PredicateKey,

    /// Chunk transitions that we've already proven.
    input_chunks: &'a [ArchivedChunkInput],

    /// Pre-state needed for processing and verifying the update transitions.
    raw_partial_pre_state: &'a [u8],
}

impl<'a, E: ExecutionEnvironment> EeVerificationInput<'a, E> {
    /// Constructs a new instance.
    ///
    /// The input chunk transitions MUST already be verified.
    pub fn new(
        ee: &'a E,
        chunk_predicate_key: &'a PredicateKey,
        input_chunks: &'a [ArchivedChunkInput],
        raw_partial_pre_state: &'a [u8],
    ) -> Self {
        Self {
            ee,
            chunk_predicate_key,
            input_chunks,
            raw_partial_pre_state,
        }
    }

    pub fn ee(&self) -> &'a E {
        self.ee
    }

    pub fn chunk_predicate_key(&self) -> &'a PredicateKey {
        self.chunk_predicate_key
    }

    pub fn input_chunks(&self) -> &'a [ArchivedChunkInput] {
        self.input_chunks
    }

    pub fn raw_partial_pre_state(&self) -> &'a [u8] {
        self.raw_partial_pre_state
    }
}

/// Verification state for EE accounts.
///
/// This type tracks all verification-related state during update processing,
/// including balance bookkeeping, pending commits, outputs, and references to
/// the private input data needed for chain segment verification.
#[expect(missing_debug_implementations, reason = "E may not implement Debug")]
pub struct EeVerificationState<'a, E: ExecutionEnvironment> {
    /// Execution environment for block execution.
    ee: &'a E,

    /// Predicate used for verifying chunk proofs.
    chunk_predicate_key: &'a PredicateKey,

    /// Current verified chain tip.
    cur_verified_exec_blkid: Hash,

    /// Tracks the total value sent in this update.
    ///
    /// Not sure why we're doing this after all.
    total_val_sent: BitcoinAmount,

    /// Tracks current balance.
    ///
    /// This is the snark account's balance on the orchestration layer, as an
    /// additional check to ensure we don't permit invalid updates.  It is still
    /// incremented when we receive messages we don't understand.
    cur_balance: BitcoinAmount,

    /// Outputs we expect to have.
    expected_outputs: UpdateOutputs,

    /// Recorded outputs we'll check later.
    accumulated_outputs: UpdateOutputs,

    /// Chunk transitions to verify.
    input_chunks: &'a [ArchivedChunkInput],

    /// Partial pre-state corresponding to the last verified block.
    // TODO use this to support DA
    raw_partial_pre_state: &'a [u8],
}

impl<'a, E: ExecutionEnvironment> EeVerificationState<'a, E> {
    /// Constructs a verification state using the account's initial state as a
    /// reference, along with the verification input data.
    pub fn new_from_state(
        ee: &'a E,
        chunk_predicate_key: &'a PredicateKey,
        state: &EeAccountState,
        expected_outputs: UpdateOutputs,
        input_chunks: &'a [ArchivedChunkInput],
        raw_partial_pre_state: &'a [u8],
    ) -> Self {
        Self {
            ee,
            chunk_predicate_key,
            cur_verified_exec_blkid: state.last_exec_blkid(),
            total_val_sent: 0.into(),
            cur_balance: state.tracked_balance(),
            expected_outputs,
            accumulated_outputs: UpdateOutputs::new_empty(),
            input_chunks,
            raw_partial_pre_state,
        }
    }

    /// Returns the execution environment.
    pub fn exec_env(&self) -> &'a E {
        self.ee
    }

    /// Returns the predkey used to verify chunk transition proofs.
    pub fn chunk_predicate_key(&self) -> &'a PredicateKey {
        self.chunk_predicate_key
    }

    pub fn cur_verified_exec_blkid(&self) -> Hash {
        self.cur_verified_exec_blkid
    }

    pub fn cur_balance(&self) -> BitcoinAmount {
        self.cur_balance
    }

    /// Increases our verification state tracked balance.
    ///
    /// This is intended for when we accept a message.
    pub fn accept_funds(&mut self, amt: BitcoinAmount) -> EnvResult<()> {
        self.cur_balance = self
            .cur_balance
            .checked_add(amt)
            .ok_or(EnvError::BalanceOverflow)?;
        Ok(())
    }

    /// Returns the raw partial pre-state.
    pub fn raw_partial_pre_state(&self) -> &'a [u8] {
        self.raw_partial_pre_state
    }

    /// Appends a package block's outputs into the pending outputs being
    /// built internally. This way we can compare it against the update op data
    /// later.
    ///
    /// # Errors
    ///
    /// If this results in overflowing buffers, then returns an error and leaves
    /// us in a dirty state where we should abort anyways.
    pub(crate) fn merge_new_outputs(&mut self, outputs: &ExecOutputs) -> EnvResult<()> {
        // Annoying thing to do checked summation.
        let sent_now = BitcoinAmount::sum(
            outputs.output_transfers().iter().map(|e| e.value()).chain(
                outputs
                    .output_messages()
                    .iter()
                    .map(|e| e.payload().value()),
            ),
        );

        // Update the balance before anything else, throw an error for insufficient funds.
        self.cur_balance = self
            .cur_balance
            .checked_sub(sent_now)
            .ok_or(EnvError::InsufficientFunds)?;

        // Just merge the entries into the buffer. This is a little more
        // complicated than it really is because we have to convert between two
        // sets of similar types that are separately defined to avoid semantic
        // confusion because they do refer to different concepts.
        self.accumulated_outputs
            .try_extend_transfers(
                outputs
                    .output_transfers()
                    .iter()
                    .map(|e| OutputTransfer::new(e.dest(), e.value())),
            )
            .map_err(|_| EnvError::OutputOverflow)?;

        self.accumulated_outputs
            .try_extend_messages(
                outputs
                    .output_messages()
                    .iter()
                    .map(|e| OutputMessage::new(e.dest(), e.payload().clone())),
            )
            .map_err(|_| EnvError::OutputOverflow)?;

        // Finally update the total_val_sent field.
        self.total_val_sent = self
            .total_val_sent
            .checked_add(sent_now)
            .ok_or(EnvError::OutputOverflow)?;

        Ok(())
    }

    /// Processes a single decoded chunk transition: validates chain linkage,
    /// matches inputs against pending inputs, merges outputs, advances tip.
    ///
    /// Separated from proof verification for independent testability.
    pub fn process_decoded_transition(
        &mut self,
        transition: &ChunkTransition,
        pending_inp_tracker: &mut SequenceTracker<'_, PendingInputEntry>,
    ) -> EnvResult<()> {
        // Chain linkage: parent must match current verified tip.
        if transition.parent_exec_blkid() != self.cur_verified_exec_blkid {
            return Err(EnvError::MismatchedChainSegment);
        }

        // Match inputs in the transition with our pending inputs.
        //
        // Each chunk deposit must match the next pending input in order by
        // type.
        for deposit in transition.inputs().subject_deposits() {
            // Consume the input.
            pending_inp_tracker
                .consume_input_with(|pending| {
                    matches!(
                        pending,
                        PendingInputEntry::Deposit(expected) if deposit == expected,
                    )
                })
                .map_err(|_| EnvError::InconsistentChunkIo)?;
        }

        // Merge outputs into accumulated state.
        self.merge_new_outputs(transition.outputs())?;

        // Advance the verified tip.
        self.cur_verified_exec_blkid = transition.tip_exec_blkid();

        Ok(())
    }

    /// Verifies all chunk transitions against the account's predicate key,
    /// checks chain linkage, matches inputs against pending inputs, and
    /// merges outputs.
    pub(crate) fn process_chunks_on_acct(
        &mut self,
        state: &EeAccountState,
        extra_data: &UpdateExtraData,
    ) -> EnvResult<()> {
        let mut pending_inp_tracker = SequenceTracker::new(state.pending_inputs());

        // Loop through all the chunks and verify them.
        for chunk in self.input_chunks {
            self.chunk_predicate_key()
                .verify_claim_witness(chunk.chunk_transition_ssz(), chunk.proof())
                .map_err(|_| EnvError::InvalidChunkProof)?;

            // Decode the transition for linkage, input matching, and outputs.
            let transition = chunk
                .try_decode_chunk_transition()
                .map_err(|_| EnvError::MalformedChainSegment)?;

            // Process the decoded transition.
            self.process_decoded_transition(&transition, &mut pending_inp_tracker)?;
        }

        // Check that the number of consumed pending inputs matches what
        // extra_data claims were processed.
        if pending_inp_tracker.consumed() != *extra_data.processed_inputs() as usize {
            return Err(EnvError::InconsistentChunkIo);
        }

        Ok(())
    }

    /// Final checks to see if there's anything in the verification state that
    /// were supposed to have been dealt with but weren't.
    pub(crate) fn check_obligations(&self) -> EnvResult<()> {
        // Check that the expected outputs match the ones we accumulated.
        if self.expected_outputs != self.accumulated_outputs {
            return Err(EnvError::UnsatisfiedObligations(
                "expected and accumulated outputs mismatch",
            ));
        }

        // Maybe more in the future.

        Ok(())
    }
}

/// Manual `Clone` impl to avoid requiring `E: Clone` (we only hold `&'a E`).
impl<'a, E: ExecutionEnvironment> Clone for EeVerificationState<'a, E> {
    fn clone(&self) -> Self {
        Self {
            ee: self.ee,
            chunk_predicate_key: self.chunk_predicate_key,
            cur_verified_exec_blkid: self.cur_verified_exec_blkid,
            total_val_sent: self.total_val_sent,
            cur_balance: self.cur_balance,
            expected_outputs: self.expected_outputs.clone(),
            accumulated_outputs: self.accumulated_outputs.clone(),
            input_chunks: self.input_chunks,
            raw_partial_pre_state: self.raw_partial_pre_state,
        }
    }
}
