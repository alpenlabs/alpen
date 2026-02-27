//! EE-specific snark account program implementation.
//!
//! This module provides the [`EeSnarkAccountProgram`] struct, which implements
//! the [`SnarkAccountProgram`] and [`SnarkAccountProgramVerification`] traits
//! for the EE account type.

use strata_ee_acct_types::{
    DecodedEeMessageData, EeAccountState, EnvError, ExecutionEnvironment, PendingInputEntry,
    UpdateExtraData,
};
use strata_ee_chain_types::SubjectDepositData;
use strata_snark_acct_runtime::*;

use crate::verification_state::{EeVerificationInput, EeVerificationState};

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
    ) -> ProgramResult<(), Self::Error> {
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

    fn pre_finalize_state(
        &self,
        state: &mut Self::State,
        extra_data: &Self::ExtraData,
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

    fn start_verification<'i, 'u>(
        &self,
        state: &Self::State,
        vinput: Self::VInput<'i>,
        ulinfo: UpdateLedgerInfo<'u>,
    ) -> ProgramResult<Self::VState<'i>, Self::Error> {
        Ok(EeVerificationState::new_from_state(
            vinput.ee(),
            state,
            ulinfo.outputs().clone(), // ugh
            vinput.input_chunks(),
            vinput.raw_partial_pre_state(),
        ))
    }

    fn verify_coinput<'a>(
        &self,
        _state: &Self::State,
        _vstate: &mut Self::VState<'a>,
        _msg: &InputMessage<Self::Msg>,
        coinput: &[u8],
    ) -> ProgramResult<(), Self::Error> {
        // For both Valid and Unknown messages, require empty coinput.
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
        // Process and verify all chunks sequentially.
        vstate.process_chunks_on_acct(state, extra_data)?;

        // Make sure the state matches the extra data.
        if state.last_exec_blkid() != *extra_data.new_tip_blkid() {
            return Err(ProgramError::InvalidExtraData);
        }

        // Make sure the state matches what we verified.
        if state.last_exec_blkid() != vstate.cur_verified_exec_blkid() {
            return Err(ProgramError::InvalidExtraData);
        }

        // Check the other internal obligations.
        vstate
            .check_obligations()
            .map_err(|_| ProgramError::UnsatisfiedObligations)?;

        Ok(())
    }
}

/// Applies state changes from a decoded EE message.
pub(crate) fn apply_decoded_message(
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
