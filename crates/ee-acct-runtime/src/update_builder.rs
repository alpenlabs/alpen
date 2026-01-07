//! Builder for constructing update operations for testing.

use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{
    CommitChainSegment, EeAccountState, ExecBlock, ExecutionEnvironment, UpdateExtraData,
};
use strata_snark_acct_types::{
    LedgerRefs, MAX_MESSAGES, MAX_TRANSFERS, MessageEntry, OutputMessage, OutputTransfer,
    ProofState, UpdateOperationData, UpdateOutputs,
};

use crate::{
    builder_errors::BuilderResult,
    private_input::SharedPrivateInput,
    update_processing::{MsgData, apply_message},
};

/// Builder for constructing complete update operations.
///
/// This helps assemble all the pieces needed to call
/// `verify_and_apply_update_operation` in tests. It tracks the account state
/// and accumulates changes as messages are processed and segments are added.
//#[expect(missing_debug_implementations, reason = "clippy is wrong")]
#[derive(Debug)]
pub struct UpdateBuilder {
    seq_no: u64,
    current_state: EeAccountState,
    processed_messages: Vec<MessageEntry>,
    message_coinputs: Vec<Vec<u8>>,
    ledger_refs: LedgerRefs,
    accumulated_outputs: UpdateOutputs,
    commit_segments: Vec<CommitChainSegment>,
    total_inputs_processed: usize,
    total_fincls_processed: usize,
}

impl UpdateBuilder {
    /// Creates a new update builder with a sequence number and initial state.
    pub fn new(seq_no: u64, initial_state: EeAccountState) -> Self {
        Self {
            seq_no,
            current_state: initial_state,
            processed_messages: Vec::new(),
            message_coinputs: Vec::new(),
            ledger_refs: LedgerRefs::new_empty(),
            accumulated_outputs: UpdateOutputs::new_empty(),
            commit_segments: Vec::new(),
            total_inputs_processed: 0,
            total_fincls_processed: 0,
        }
    }

    /// Accepts and processes a message, updating the account state.
    ///
    /// This applies the message's effects to the current state (adding balance,
    /// queuing pending inputs) just as it would during normal update processing.
    /// For now, coinput verification is not performed (just stored).
    pub fn accept_message(
        mut self,
        message: MessageEntry,
        coinput: Vec<u8>,
    ) -> BuilderResult<Self> {
        // Decode the message
        let msg_data = MsgData::from_entry(&message)?;

        // Apply message effects to the state using the same function as normal processing
        apply_message(&mut self.current_state, &msg_data)?;

        // Store the message and coinput for later
        self.processed_messages.push(message);
        self.message_coinputs.push(coinput);

        Ok(self)
    }

    /// Sets the ledger references for this update.
    pub fn with_ledger_refs(mut self, refs: LedgerRefs) -> Self {
        self.ledger_refs = refs;
        self
    }

    /// Adds a chain segment to the update.
    ///
    /// This automatically accumulates the outputs from all blocks in the segment
    /// and removes consumed inputs from the current state.
    pub fn add_segment(mut self, segment: CommitChainSegment) -> Self {
        // Track total inputs consumed across all blocks
        let mut total_inputs_consumed = 0;

        // Accumulate outputs from all blocks in the segment
        for block in segment.blocks() {
            let block_package = block.package();
            let block_outputs = block_package.outputs();
            let block_inputs = block_package.inputs();

            // Count inputs consumed by this block
            total_inputs_consumed += block_inputs.total_inputs();

            // Add transfers
            self.accumulated_outputs
                .try_extend_transfers(
                    block_outputs
                        .output_transfers()
                        .iter()
                        .map(|t| OutputTransfer::new(t.dest(), t.value())),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "transfers capacity exceeded in test builder: {e}. Current: {}, Adding: {}, Max: {}",
                        self.accumulated_outputs.transfers().len(),
                        block_outputs.output_transfers().len(),
                        MAX_TRANSFERS
                    )
                });

            // Add messages (converting payload types)
            self.accumulated_outputs
                .try_extend_messages(
                    block_outputs
                        .output_messages()
                        .iter()
                        .map(|m| OutputMessage::new(m.dest(), m.payload().clone())),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "messages capacity exceeded in test builder: {e}. Current: {}, Adding: {}, Max: {}",
                        self.accumulated_outputs.messages().len(),
                        block_outputs.output_messages().len(),
                        MAX_MESSAGES
                    )
                });
        }

        // Remove consumed inputs from the current state
        self.current_state
            .remove_pending_inputs(total_inputs_consumed);

        // Track total inputs processed
        self.total_inputs_processed += total_inputs_consumed;

        self.commit_segments.push(segment);
        self
    }

    /// Sets the number of forced inclusions processed.
    pub fn with_processed_fincls(mut self, count: usize) -> Self {
        self.total_fincls_processed = count;
        self
    }

    /// Builds the update operation data and shared private input.
    ///
    /// Requires the initial state (before any changes), previous header, and
    /// partial state for the shared private input. The extra data is
    /// automatically constructed from:
    /// - new_tip_blkid: the last block ID from the last segment
    /// - processed_inputs: calculated from segments (total inputs consumed)
    /// - processed_fincls: set via with_processed_fincls
    pub fn build<E: ExecutionEnvironment>(
        mut self,
        initial_state: &EeAccountState,
        prev_header: &<E::Block as ExecBlock>::Header,
        prev_partial_state: &E::PartialState,
    ) -> BuilderResult<(UpdateOperationData, SharedPrivateInput, Vec<Vec<u8>>)> {
        // Determine the new tip block ID from the last segment
        let new_tip_blkid = self
            .commit_segments
            .last()
            .and_then(|seg| seg.new_exec_tip_blkid())
            .unwrap_or(initial_state.last_exec_blkid());

        // Construct the extra data using tracked values
        let extra_data = UpdateExtraData::new(
            new_tip_blkid,
            self.total_inputs_processed as u32,
            self.total_fincls_processed as u32,
        );

        // Encode the extra data
        let extra_data_buf = encode_to_vec(&extra_data)?;

        // Compute the new ProofState
        let new_state = compute_new_proof_state(
            &mut self.current_state,
            new_tip_blkid,
            self.processed_messages.len(),
        );

        // Encode the previous header and partial state
        let prev_header_buf = encode_to_vec(prev_header)?;
        let prev_partial_state_buf = encode_to_vec(prev_partial_state)?;

        // Build the operation data
        let operation = UpdateOperationData::new(
            self.seq_no,
            new_state,
            self.processed_messages,
            self.ledger_refs,
            self.accumulated_outputs,
            extra_data_buf,
        );

        // Build the shared private input
        let shared_private = SharedPrivateInput::new(
            self.commit_segments,
            prev_header_buf,
            prev_partial_state_buf,
        );

        Ok((operation, shared_private, self.message_coinputs))
    }

    /// Returns the current state.
    pub fn current_state(&self) -> &EeAccountState {
        &self.current_state
    }

    /// Returns the accumulated outputs so far.
    pub fn accumulated_outputs(&self) -> &UpdateOutputs {
        &self.accumulated_outputs
    }
}

/// Helper function to compute the new ProofState after an update.
///
/// This function:
/// 1. Updates the `last_exec_blkid` in the state (to match `apply_final_update_changes`)
/// 2. Computes the `inner_state_root` using SSZ tree_hash_root
/// 3. Calculates `next_msg_read_idx` (assumes prev_next_msg_read_idx = 0 for now)
/// 4. Returns the ProofState with both values
fn compute_new_proof_state(
    state: &mut EeAccountState,
    new_tip_blkid: Hash,
    processed_messages_count: usize,
) -> ProofState {
    // Update last_exec_blkid before computing hash (matches apply_final_update_changes)
    state.set_last_exec_blkid(new_tip_blkid);

    // Compute the inner_state_root using tree_hash_root
    let computed_hash = tree_hash::TreeHash::<tree_hash::Sha256Hasher>::tree_hash_root(state);
    let inner_state_root = Hash::from(computed_hash.0);

    // For now, assume prev_next_msg_read_idx = 0
    // TODO: Need to pass prev_next_msg_read_idx to UpdateBuilder::new()
    let next_msg_read_idx = processed_messages_count as u64;

    ProofState::new(inner_state_root, next_msg_read_idx)
}
