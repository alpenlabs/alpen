//! EE-specific snark account program implementation.
//!
//! This module provides the [`EeSnarkAccountProgram`] struct, which implements
//! the [`SnarkAccountProgram`] and [`SnarkAccountProgramVerification`] traits
//! for the EE account type.

use strata_ee_acct_types::{
    DecodedEeMessageData, EeAccountState, EnvError, ExecutionEnvironment, PendingInputEntry,
    UpdateExtraData,
};
use strata_ee_chain_types::{SequenceTracker, SubjectDepositData};
use strata_snark_acct_runtime::*;

use crate::{
    commit::PendingCommit,
    exec_processing::process_segments,
    verification_state::{EeVerificationInput, EeVerificationState},
};

/// Snark account program for execution environments.
///
/// The type parameter `E` is the execution environment type used for block
/// execution during verification.
#[derive(Debug)]
pub struct EeSnarkAccountProgram<E: ExecutionEnvironment> {
    _marker: std::marker::PhantomData<E>,
}

impl<E: ExecutionEnvironment> EeSnarkAccountProgram<E> {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E: ExecutionEnvironment> SnarkAccountProgram for EeSnarkAccountProgram<E> {
    type State = EeAccountState;
    type Msg = DecodedEeMessageData;
    type ExtraData = UpdateExtraData;
    type Error = EnvError;

    fn process_message(
        &self,
        state: &mut Self::State,
        msg: InputMessage<Self::Msg>,
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // Just call out to the generic function used elsewhere.
        process_ee_message(state, msg)
    }

    fn finalize_state(
        &self,
        state: &mut Self::State,
        extra_data: Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // Update final execution head block.
        state.set_last_exec_blkid(*extra_data.new_tip_blkid());

        // Update queues.
        state.remove_pending_inputs(*extra_data.processed_inputs() as usize);
        state.remove_pending_fincls(*extra_data.processed_fincls() as usize);

        Ok(())
    }
}

impl<E: ExecutionEnvironment> SnarkAccountProgramVerification for EeSnarkAccountProgram<E> {
    type VState<'a> = EeVerificationState<'a, E>;
    type VInput<'a> = EeVerificationInput<'a, E>;

    fn start_verification<'a>(
        &self,
        state: &Self::State,
        _extra_data: &Self::ExtraData,
        vinput: Self::VInput<'a>,
    ) -> ProgramResult<Self::VState<'a>, Self::Error> {
        Ok(EeVerificationState::new_from_state(
            state,
            vinput.ee,
            vinput.shared_private.commit_data(),
            vinput.shared_private.raw_prev_header(),
            vinput.shared_private.raw_partial_pre_state(),
        ))
    }

    fn verify_coinput<'a>(
        &self,
        _state: &Self::State,
        _vstate: &mut Self::VState<'a>,
        _msg: &InputMessage<Self::Msg>,
        coinput: &[u8],
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // For both Valid and Unknown messages: require empty coinput.
        // We don't need any message coinputs for the EE right now.
        if !coinput.is_empty() {
            return Err(ProgramError::MalformedCoinput);
        }

        Ok(())
    }

    fn finalize_verification<'a>(
        &self,
        state: &Self::State,
        mut vstate: Self::VState<'a>,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // Add pending commit if tip changed and it's not the tip commit already.
        let is_ntip_changed = *extra_data.new_tip_blkid() != state.last_exec_blkid();
        let is_ntip_committed = vstate
            .pending_commits()
            .last()
            .is_some_and(|pc| pc.new_tip_exec_blkid() == *extra_data.new_tip_blkid());
        if is_ntip_changed && !is_ntip_committed {
            vstate.add_pending_commit(PendingCommit::new(*extra_data.new_tip_blkid()));
        }

        // Validate that we got chain segments corresponding to the pending commits.
        if vstate.pending_commits().len() != vstate.commit_data().len() {
            return Err(EnvError::MismatchedChainSegment.into());
        }

        // Process chain segments using private input from vstate.
        let mut input_tracker = SequenceTracker::new(state.pending_inputs());
        process_segments(&mut vstate, &mut input_tracker).map_err(ProgramError::Internal)?;

        // Check that the inputs we consumed match the number we're saying were
        // consumed.
        if input_tracker.consumed() != *extra_data.processed_inputs() as usize {
            return Err(EnvError::MismatchedChainSegment.into());
        }

        vstate
            .check_obligations()
            .map_err(|_| ProgramError::UnsatisfiedObligations)?;

        Ok(())
    }
}

/// Applies state changes from a decoded EE message.
pub fn apply_decoded_message(
    state: &mut EeAccountState,
    msg: &DecodedEeMessageData,
    value: strata_acct_types::BitcoinAmount,
) -> ProgramResult<(), EnvError> {
    match msg {
        DecodedEeMessageData::Deposit(data) => {
            // Create deposit data with the actual value from the message.
            let deposit_data = SubjectDepositData::new(*data.dest_subject(), value);
            state.add_pending_input(PendingInputEntry::Deposit(deposit_data));
        }

        DecodedEeMessageData::SubjTransfer(_data) => {
            // TODO handle subject transfers
        }

        DecodedEeMessageData::Commit(_data) => {
            // Just ignore this one for now because we're not handling it.
            // TODO support this
        }
    }

    Ok(())
}

/// Processes an input message, updating state accordingly.
// why is this a separate function?
pub fn process_ee_message(
    state: &mut EeAccountState,
    msg: InputMessage<DecodedEeMessageData>,
) -> ProgramResult<(), EnvError> {
    // Add value to tracked balance, always do this.
    if !msg.meta().value().is_zero() {
        state.add_tracked_balance(msg.meta().value());
    }

    // If we recognize it, then we have to do something with it.
    if let InputMessage::Valid(meta, decoded_msg) = msg {
        apply_decoded_message(state, &decoded_msg, meta.value())?;
    }

    Ok(())
}

/// Finalizes state after processing messages.
///
/// This is a standalone function for use in the unconditional path where
/// a generic program instance is not needed.
pub fn finalize_ee_state(
    state: &mut EeAccountState,
    extra_data: &UpdateExtraData,
) -> ProgramResult<(), EnvError> {
    // Update final execution head block.
    state.set_last_exec_blkid(*extra_data.new_tip_blkid());

    // Update queues.
    state.remove_pending_inputs(*extra_data.processed_inputs() as usize);
    state.remove_pending_fincls(*extra_data.processed_fincls() as usize);

    Ok(())
}
