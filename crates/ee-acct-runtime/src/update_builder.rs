//! EE-specific update builder with chunk-aware interface.
//!
//! Wraps the generic snark account builder and tracks chain tip and pending
//! inputs, allowing the consumer to query available inputs and accept
//! validated [`ChunkTransition`]s.

use strata_acct_types::Hash;
use strata_ee_acct_types::{
    EeAccountState, ExecutionEnvironment, PendingInputEntry, UpdateExtraData,
};
use strata_ee_chain_types::ChunkTransition;
use strata_snark_acct_runtime::UpdateBuilder as GenericUpdateBuilder;
use strata_snark_acct_types::{
    LedgerRefs, MAX_MESSAGES, MAX_TRANSFERS, MessageEntry, OutputMessage, OutputTransfer,
    SnarkAccountState, UpdateOperationData, UpdateOutputs,
};

use crate::{
    builder_errors::{BuilderError, BuilderResult},
    ee_program::EeSnarkAccountProgram,
    verification_state::EeVerificationInput,
};

/// EE update builder with chunk-aware interface.
///
/// Tracks the execution chain tip and pending input queue internally,
/// validates [`ChunkTransition`]s against this state, and accumulates
/// outputs. The consumer can query [`remaining_pending_inputs`] and
/// [`cur_tip_blkid`] to construct valid transitions.
#[expect(missing_debug_implementations, reason = "E may not implement Debug")]
pub struct UpdateBuilder<'i, E: ExecutionEnvironment> {
    inner: GenericUpdateBuilder<'i, EeSnarkAccountProgram<E>>,
    seq_no: u64,

    /// Current chain tip, advanced as chunks are accepted.
    cur_tip_blkid: Hash,

    /// Snapshot of pending inputs after message processing. Not mutated —
    /// used to validate chunks and let the consumer query remaining inputs.
    pending_inputs: Vec<PendingInputEntry>,

    /// How many pending inputs have been consumed by accepted chunks.
    inputs_consumed: usize,

    /// Number of forced inclusions processed.
    fincls_processed: usize,
}

impl<'i, E: ExecutionEnvironment> UpdateBuilder<'i, E> {
    /// Creates a new EE update builder.
    ///
    /// Processes all messages immediately (with empty coinputs, as required by
    /// the EE program), then snapshots the resulting pending inputs and chain
    /// tip for chunk validation.
    pub fn new(
        seq_no: u64,
        snark_state: SnarkAccountState,
        initial_state: EeAccountState,
        messages: Vec<MessageEntry>,
        vinput: EeVerificationInput<'i, E>,
    ) -> BuilderResult<Self> {
        let program = EeSnarkAccountProgram::new();

        let mut inner = GenericUpdateBuilder::new(
            program,
            snark_state,
            initial_state,
            messages,
            vinput,
            LedgerRefs::new_empty(),
            UpdateOutputs::new_empty(),
        )?;

        // EE always uses empty coinputs — provide them all now.
        inner.provide_empty_coinputs()?;

        // Snapshot state after message processing (deposits are now pending).
        let pending_inputs = inner.current_state().pending_inputs().to_vec();
        let cur_tip_blkid = inner.current_state().last_exec_blkid();

        Ok(Self {
            inner,
            seq_no,
            cur_tip_blkid,
            pending_inputs,
            inputs_consumed: 0,
            fincls_processed: 0,
        })
    }

    /// Returns the current chain tip block ID.
    ///
    /// A chunk's `parent_exec_blkid` must equal this value.
    pub fn cur_tip_blkid(&self) -> Hash {
        self.cur_tip_blkid
    }

    /// Returns the remaining pending inputs that haven't been consumed by
    /// chunks yet.
    ///
    /// A chunk's deposits must match these in order.
    pub fn remaining_pending_inputs(&self) -> &[PendingInputEntry] {
        &self.pending_inputs[self.inputs_consumed..]
    }

    /// Returns the next `count` pending inputs available for consumption.
    ///
    /// Returns fewer than `count` if not enough remain.
    pub fn next_pending_inputs(&self, count: usize) -> &[PendingInputEntry] {
        let remaining = self.remaining_pending_inputs();
        let n = count.min(remaining.len());
        &remaining[..n]
    }

    /// Returns the total number of pending inputs remaining.
    pub fn remaining_input_count(&self) -> usize {
        self.pending_inputs.len() - self.inputs_consumed
    }

    /// Accepts a chunk transition, validating chain linkage and input matching.
    ///
    /// The transition's `parent_exec_blkid` must equal [`cur_tip_blkid`], and
    /// its deposits must match the next pending inputs in order.
    ///
    /// On success, advances the chain tip, accumulates outputs, and tracks
    /// consumed input count.
    pub fn accept_chunk_transition(&mut self, transition: &ChunkTransition) -> BuilderResult<()> {
        // 1. Validate chain linkage.
        if transition.parent_exec_blkid() != self.cur_tip_blkid {
            return Err(BuilderError::ChainLinkage {
                expected: self.cur_tip_blkid,
                parent: transition.parent_exec_blkid(),
            });
        }

        // 2. Validate input matching against pending inputs.
        let deposits = transition.inputs().subject_deposits();
        let remaining = self.remaining_pending_inputs();

        if deposits.len() > remaining.len() {
            return Err(BuilderError::InputMismatch {
                position: remaining.len(),
            });
        }

        for (i, deposit) in deposits.iter().enumerate() {
            match &remaining[i] {
                PendingInputEntry::Deposit(expected) if deposit == expected => {}
                _ => {
                    return Err(BuilderError::InputMismatch {
                        position: self.inputs_consumed + i,
                    });
                }
            }
        }

        // 3. Merge outputs.
        let outputs = transition.outputs();

        self.inner
            .outputs_mut()
            .try_extend_transfers(
                outputs
                    .output_transfers()
                    .iter()
                    .map(|t| OutputTransfer::new(t.dest(), t.value())),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "transfers capacity exceeded in EE builder: {e}. \
                     Current: {}, Adding: {}, Max: {}",
                    self.inner.outputs().transfers().len(),
                    outputs.output_transfers().len(),
                    MAX_TRANSFERS
                )
            });

        self.inner
            .outputs_mut()
            .try_extend_messages(
                outputs
                    .output_messages()
                    .iter()
                    .map(|m| OutputMessage::new(m.dest(), m.payload().clone())),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "messages capacity exceeded in EE builder: {e}. \
                     Current: {}, Adding: {}, Max: {}",
                    self.inner.outputs().messages().len(),
                    outputs.output_messages().len(),
                    MAX_MESSAGES
                )
            });

        // 4. Advance tip and consumed count.
        self.cur_tip_blkid = transition.tip_exec_blkid();
        self.inputs_consumed += deposits.len();

        Ok(())
    }

    /// Sets the number of forced inclusions processed.
    pub fn with_processed_fincls(mut self, count: usize) -> Self {
        self.fincls_processed = count;
        self
    }

    /// Builds the update, returning operation data and raw coinputs.
    ///
    /// Sets [`UpdateExtraData`] with the current tip, consumed input count,
    /// and forced inclusion count, then delegates to the generic builder.
    ///
    /// Uses the unverified finalization path since the builder has already
    /// validated chunk transitions locally via [`accept_chunk_transition`].
    pub fn build(self) -> BuilderResult<(UpdateOperationData, Vec<Vec<u8>>)> {
        let extra_data = UpdateExtraData::new(
            self.cur_tip_blkid,
            self.inputs_consumed as u32,
            self.fincls_processed as u32,
        );

        let (op, coinputs) = self
            .inner
            .build_operation_data_unverified(self.seq_no, extra_data)?;
        Ok((op, coinputs))
    }

    /// Returns the current account state.
    pub fn current_state(&self) -> &EeAccountState {
        self.inner.current_state()
    }

    /// Returns the accumulated outputs so far.
    pub fn outputs(&self) -> &UpdateOutputs {
        self.inner.outputs()
    }
}
