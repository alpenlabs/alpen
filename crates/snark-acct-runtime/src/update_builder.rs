//! Generic verification-aware update builder for snark account programs.
//!
//! Provides [`UpdateBuilder`], a generic builder that orchestrates the
//! verification and state mutation lifecycle of a snark account update.  It
//! processes messages one at a time (with coinput provision) and then finalizes
//! to produce either a [`PrivateInput`] (for proof generation),
//! [`UpdateManifest`] (for DA reconstruction), or [`UpdateOperationData`] (for
//! submitting to the OL).

use ssz::Encode;
use strata_codec::encode_to_vec;
use strata_snark_acct_types::{
    LedgerRefs, MessageEntry, ProofState, SnarkAccountState, UpdateManifest, UpdateOperationData,
    UpdateOutputs,
};

use crate::{
    IInnerState, InputMessage, UpdateLedgerInfo,
    errors::{ProgramError, ProgramResult},
    private_input::{Coinput, PrivateInput},
    traits::SnarkAccountProgramVerification,
};

/// Generic verification-aware update builder.
///
/// Orchestrates the full lifecycle of constructing a snark account update:
/// construct, provide coinputs for each message, then finalize into one of
/// several output formats.
#[expect(missing_debug_implementations, reason = "P may not implement Debug")]
pub struct UpdateBuilder<'i, P: SnarkAccountProgramVerification> {
    program: P,
    snark_state: SnarkAccountState,
    pre_state: P::State,
    cur_state: P::State,
    vstate: P::VState<'i>,
    messages: Vec<MessageEntry>,
    coinputs: Vec<Vec<u8>>,
    next_msg_idx: usize,
    ledger_refs: LedgerRefs,
    outputs: UpdateOutputs,
}

impl<'i, P: SnarkAccountProgramVerification> UpdateBuilder<'i, P> {
    /// Creates a new update builder.
    ///
    /// Calls `start_verification` and `start_update` on the program, but does
    /// NOT process any messages yet. Messages are added incrementally via
    /// [`Self::add_message`] or [`Self::add_message_with_coinput`], then processed via
    /// [`Self::provide_coinput`].
    pub fn new(
        program: P,
        snark_state: SnarkAccountState,
        state: P::State,
        vinput: P::VInput<'i>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
    ) -> ProgramResult<Self, P::Error> {
        let pre_state = state.clone();
        let mut current_state = state;

        // Verify pre-state matches snark account state.
        if current_state.compute_state_root() != snark_state.proof_state().inner_state() {
            return Err(ProgramError::MismatchedPreState);
        }

        // Create update ledger info and start verification.
        let ulinfo = UpdateLedgerInfo::new(&ledger_refs, &outputs);
        let vstate = program.start_verification(&current_state, vinput, ulinfo)?;

        // Start update (initial state changes before messages).
        program.start_update(&mut current_state)?;

        Ok(Self {
            program,
            snark_state,
            pre_state,
            cur_state: current_state,
            vstate,
            messages: Vec::new(),
            coinputs: Vec::new(),
            next_msg_idx: 0,
            ledger_refs,
            outputs,
        })
    }

    /// Appends a message without processing it.
    ///
    /// The caller must later call [`Self::provide_coinput`] to process this message.
    pub fn add_message(&mut self, msg: MessageEntry) {
        self.messages.push(msg);
    }

    /// Appends a message and immediately processes it with the given coinput.
    ///
    /// Equivalent to calling [`Self::add_message`] followed by [`Self::provide_coinput`].
    pub fn add_message_with_coinput(
        &mut self,
        msg: MessageEntry,
        coinput: Vec<u8>,
    ) -> ProgramResult<(), P::Error> {
        self.messages.push(msg);
        self.provide_coinput(coinput)
    }

    /// Returns the current inner state of the snark account, accounting for the
    /// messages processed.
    pub fn cur_state(&self) -> &P::State {
        &self.cur_state
    }

    /// Returns a mutable reference to the current state.  Be careful with what
    /// you do with this!
    pub fn cur_state_mut(&mut self) -> &mut P::State {
        &mut self.cur_state
    }

    /// Returns the current verification state of the update, accounting for the
    /// messages processed.
    pub fn cur_vstate(&self) -> &P::VState<'i> {
        &self.vstate
    }

    /// Returns the next message entry to provide a coinput for.
    pub fn next_message_entry(&self) -> Option<&MessageEntry> {
        self.messages.get(self.next_msg_idx)
    }

    /// Returns the next message, decoded.
    pub fn next_message(&self) -> Option<InputMessage<P::Msg>> {
        self.messages
            .get(self.next_msg_idx)
            .map(InputMessage::from_msg_entry)
    }

    /// Returns a message at any index, decoded.
    pub fn message_at(&self, idx: usize) -> Option<InputMessage<P::Msg>> {
        self.messages.get(idx).map(InputMessage::from_msg_entry)
    }

    /// Returns the number of messages remaining.
    pub fn remaining_messages(&self) -> usize {
        self.messages.len().saturating_sub(self.next_msg_idx)
    }

    /// Provides a coinput for the next message.
    ///
    /// Calls `verify_coinput` then `process_message` on the program.
    pub fn provide_coinput(&mut self, coinput: Vec<u8>) -> ProgramResult<(), P::Error> {
        let idx = self.next_msg_idx;
        let msg_entry = self
            .messages
            .get(idx)
            .ok_or(ProgramError::MismatchedCoinputCount {
                expected: self.messages.len(),
                actual: idx + 1,
            })?;

        let inp_msg = InputMessage::<P::Msg>::from_msg_entry(msg_entry);

        // Unknown messages with non-empty coinput is invalid.
        if !inp_msg.is_valid() && !coinput.is_empty() {
            return Err(ProgramError::InvalidCoinput.at_msg(idx));
        }

        // Verify coinput.
        self.program
            .verify_coinput(&self.cur_state, &mut self.vstate, &inp_msg, &coinput)
            .map_err(|e| e.at_msg(idx))?;

        // Process the message.
        self.program
            .process_message(&mut self.cur_state, inp_msg)
            .map_err(|e| e.at_msg(idx))?;

        self.coinputs.push(coinput);
        self.next_msg_idx += 1;

        Ok(())
    }

    /// Provides empty coinputs for all remaining messages.
    pub fn provide_empty_coinputs(&mut self) -> ProgramResult<(), P::Error> {
        while self.next_msg_idx < self.messages.len() {
            self.provide_coinput(Vec::new())?;
        }
        Ok(())
    }

    /// Returns a reference to the outputs.
    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    /// Returns a mutable reference to the outputs.
    pub fn outputs_mut(&mut self) -> &mut UpdateOutputs {
        &mut self.outputs
    }

    /// Asserts all coinputs have been provided.
    fn assert_all_coinputs_provided(&self) -> ProgramResult<(), P::Error> {
        if self.next_msg_idx != self.messages.len() {
            return Err(ProgramError::MismatchedCoinputCount {
                expected: self.messages.len(),
                actual: self.next_msg_idx,
            });
        }
        Ok(())
    }

    /// Shared finalization: pre_finalize, finalize_verification, finalize_state.
    ///
    /// Returns the finalized state, extra data, raw pre-state, and coinputs.
    /// Clones internal state so the builder can be reused.
    fn finalize_verified(
        &mut self,
        extra_data: P::ExtraData,
    ) -> ProgramResult<FinalizedUpdate<P>, P::Error>
    where
        P::VState<'i>: Clone,
    {
        self.assert_all_coinputs_provided()?;

        let mut post_state = self.cur_state.clone();

        // Pre-finalize state.
        self.program
            .pre_finalize_state(&mut post_state, &extra_data)?;

        // Final verification (consumes a clone of vstate).
        let vstate_clone = self.vstate.clone();
        self.program
            .finalize_verification(&post_state, vstate_clone, &extra_data)?;

        // Finalize state.
        let extra_data_clone = extra_data.clone();
        self.program.finalize_state(&mut post_state, extra_data)?;

        Ok(FinalizedUpdate {
            pre_state: self.pre_state.clone(),
            post_state,
            snark_state: self.snark_state.clone(),
            extra_data: extra_data_clone,
            messages: self.messages.clone(),
            coinputs: self.coinputs.clone(),
            ledger_refs: self.ledger_refs.clone(),
            outputs: self.outputs.clone(),
        })
    }

    /// Shared finalization for the unconditional (manifest) path.
    ///
    /// Skips `finalize_verification`. Clones internal state so the builder can
    /// be reused.
    fn finalize_unverified(
        &mut self,
        extra_data: P::ExtraData,
    ) -> ProgramResult<FinalizedUpdate<P>, P::Error> {
        self.assert_all_coinputs_provided()?;

        let mut post_state = self.cur_state.clone();

        // Pre-finalize state.
        self.program
            .pre_finalize_state(&mut post_state, &extra_data)?;

        // Skip finalize_verification.

        // Finalize state.
        let extra_data_clone = extra_data.clone();
        self.program.finalize_state(&mut post_state, extra_data)?;

        Ok(FinalizedUpdate {
            pre_state: self.pre_state.clone(),
            post_state,
            snark_state: self.snark_state.clone(),
            extra_data: extra_data_clone,
            messages: self.messages.clone(),
            coinputs: self.coinputs.clone(),
            ledger_refs: self.ledger_refs.clone(),
            outputs: self.outputs.clone(),
        })
    }

    /// Builds a [`PrivateInput`] for proof generation.
    ///
    /// Calls `pre_finalize_state`, `finalize_verification`, `finalize_state`.
    /// Clones internal state so the builder can be reused.
    pub fn build_private_input(
        &mut self,
        extra_data: P::ExtraData,
    ) -> ProgramResult<PrivateInput, P::Error>
    where
        P::VState<'i>: Clone,
    {
        let finalized = self.finalize_verified(extra_data)?;
        finalized.into_private_input()
    }

    /// Builds an [`UpdateManifest`] for the unconditional (DA reconstruction)
    /// path.
    ///
    /// Calls `pre_finalize_state`, `finalize_state` (skips
    /// `finalize_verification`). Clones internal state so the builder can be
    /// reused.
    pub fn build_manifest(
        &mut self,
        extra_data: P::ExtraData,
    ) -> ProgramResult<UpdateManifest, P::Error> {
        let finalized = self.finalize_unverified(extra_data)?;
        finalized.into_manifest()
    }

    /// Builds [`UpdateOperationData`] and raw coinputs.
    ///
    /// Calls `pre_finalize_state`, `finalize_verification`, `finalize_state`.
    /// Clones internal state so the builder can be reused.
    pub fn build_operation_data(
        &mut self,
        seq_no: u64,
        extra_data: P::ExtraData,
    ) -> ProgramResult<(UpdateOperationData, Vec<Vec<u8>>), P::Error>
    where
        P::VState<'i>: Clone,
    {
        let finalized = self.finalize_verified(extra_data)?;
        finalized.into_operation_data(seq_no)
    }

    /// Builds [`UpdateOperationData`] and raw coinputs without running
    /// verification finalization.
    ///
    /// Calls `pre_finalize_state`, `finalize_state` (skips
    /// `finalize_verification`). Useful for the construction side where
    /// the builder has already validated chunks locally. Clones internal state
    /// so the builder can be reused.
    pub fn build_operation_data_unverified(
        &mut self,
        seq_no: u64,
        extra_data: P::ExtraData,
    ) -> ProgramResult<(UpdateOperationData, Vec<Vec<u8>>), P::Error> {
        let finalized = self.finalize_unverified(extra_data)?;
        finalized.into_operation_data(seq_no)
    }
}

/// Internal struct holding finalized update data.
struct FinalizedUpdate<P: SnarkAccountProgramVerification> {
    pre_state: P::State,
    post_state: P::State,
    snark_state: SnarkAccountState,
    extra_data: P::ExtraData,
    messages: Vec<MessageEntry>,
    coinputs: Vec<Vec<u8>>,
    ledger_refs: LedgerRefs,
    outputs: UpdateOutputs,
}

impl<P: SnarkAccountProgramVerification> FinalizedUpdate<P> {
    fn pre_inbox_idx(&self) -> u64 {
        self.snark_state.proof_state().next_inbox_msg_idx()
    }

    fn new_proof_state(&self) -> ProofState {
        let new_inbox_idx = self.pre_inbox_idx() + self.messages.len() as u64;
        ProofState::new(self.post_state.compute_state_root(), new_inbox_idx)
    }

    fn encode_extra_data(&self) -> ProgramResult<Vec<u8>, P::Error> {
        encode_to_vec(&self.extra_data).map_err(ProgramError::Codec)
    }

    fn into_private_input(self) -> ProgramResult<PrivateInput, P::Error> {
        let pre_inbox_idx = self.pre_inbox_idx();
        let new_proof_state = self.new_proof_state();
        let extra_data_buf = self.encode_extra_data()?;
        let raw_pre_state = self.pre_state.as_ssz_bytes();

        let coinputs = self.coinputs.into_iter().map(Coinput::new).collect();

        let pub_params = strata_snark_acct_types::UpdateProofPubParams::new(
            ProofState::new(self.pre_state.compute_state_root(), pre_inbox_idx),
            new_proof_state,
            self.messages,
            self.ledger_refs,
            self.outputs,
            extra_data_buf,
        );

        Ok(PrivateInput::new(pub_params, raw_pre_state, coinputs))
    }

    fn into_manifest(self) -> ProgramResult<UpdateManifest, P::Error> {
        let new_proof_state = self.new_proof_state();
        let extra_data_buf = self.encode_extra_data()?;

        Ok(UpdateManifest::new(
            new_proof_state,
            extra_data_buf,
            self.messages,
        ))
    }

    fn into_operation_data(
        self,
        seq_no: u64,
    ) -> ProgramResult<(UpdateOperationData, Vec<Vec<u8>>), P::Error> {
        let new_proof_state = self.new_proof_state();
        let extra_data_buf = self.encode_extra_data()?;

        let op = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            self.messages,
            self.ledger_refs,
            self.outputs,
            extra_data_buf,
        );

        Ok((op, self.coinputs))
    }
}
