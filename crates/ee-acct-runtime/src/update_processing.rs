//! High-level procedures for processing updates.
//!
//! There are two main functions here.  The first is
//! [`verify_and_apply_update_operation`], which is what we use in the proof to
//! carefully check and update the account state.  This relies on an execution
//! environment impl to be able to check the things.
//!
//! The second is [`apply_update_operation_unconditionally`], which is used
//! outside the proof, after verifying the proof, to update our view of the
//! state, presumably with information extracted from DA.  This does not require
//! understanding the execution environment.

use strata_acct_types::Hash;
#[cfg(any(test, feature = "test-utils"))]
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_codec::decode_buf_exact;
#[cfg(any(test, feature = "test-utils"))]
use strata_ee_acct_types::MessageDecodeResult;
use strata_ee_acct_types::{
    DecodedEeMessageData, EeAccountState, EnvError, EnvProgramResult, EnvResult,
    ExecutionEnvironment, UpdateExtraData,
};
use strata_snark_acct_runtime::{InputMessage, MsgMeta, ProgramError, ProgramResult};
use strata_snark_acct_types::{MessageEntry, UpdateInputData, UpdateOperationData};

use crate::{
    ee_program::{self, EeSnarkAccountProgram},
    private_input::SharedPrivateInput,
    verification_state_new::EeVerificationInput,
};

/// Parses a [`MessageEntry`] into an [`InputMessage`].
///
/// Always succeeds - if the message cannot be decoded, returns `InputMessage::Unknown`.
fn parse_input_message(entry: &MessageEntry) -> InputMessage<DecodedEeMessageData> {
    let meta = MsgMeta::new(entry.source(), entry.incl_epoch(), entry.payload_value());
    match DecodedEeMessageData::decode_raw(entry.payload_buf()) {
        Ok(msg) => InputMessage::Valid(meta, msg),
        Err(_) => InputMessage::Unknown(meta),
    }
}

/// Decoded message and its metadata.
///
/// This is primarily used for testing. For production code, use
/// [`parse_input_message`] to convert [`MessageEntry`] to [`InputMessage`].
#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug)]
pub struct MsgData {
    pub(crate) meta: MsgMeta,
    pub(crate) message: DecodedEeMessageData,
}

#[cfg(any(test, feature = "test-utils"))]
impl MsgData {
    pub(crate) fn from_entry(m: &MessageEntry) -> MessageDecodeResult<Self> {
        let message = DecodedEeMessageData::decode_raw(m.payload_buf())?;
        let meta = MsgMeta::new(m.source(), m.incl_epoch(), m.payload_value());

        Ok(Self { meta, message })
    }

    /// Creates a new `MsgData` for testing purposes.
    pub fn new_for_test(
        source: AccountId,
        incl_epoch: u32,
        value: BitcoinAmount,
        message: DecodedEeMessageData,
    ) -> Self {
        Self {
            meta: MsgMeta::new(source, incl_epoch, value),
            message,
        }
    }

    pub fn value(&self) -> BitcoinAmount {
        self.meta.value()
    }

    pub fn decoded_message(&self) -> &DecodedEeMessageData {
        &self.message
    }
}

/// Verify if an update operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
///
/// This accepts the update operation data as an argument.
pub fn verify_and_apply_update_operation<'i>(
    astate: &mut EeAccountState,
    operation: &UpdateOperationData,
    coinputs: impl IntoIterator<Item = &'i [u8]>,
    shared_private: &SharedPrivateInput,
    ee: &impl ExecutionEnvironment,
) -> EnvProgramResult<()> {
    // Parse extra data.
    let extra: UpdateExtraData =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;

    // Parse all messages. ALL messages are passed to the program (including Unknown).
    let messages = operation
        .processed_messages()
        .iter()
        .map(parse_input_message);

    // Create program.
    let program = EeSnarkAccountProgram::new();

    // Create verification input containing EE and private data.
    let vinput = EeVerificationInput::new(shared_private, ee);

    // Delegate to generic implementation.
    strata_snark_acct_runtime::verify_and_apply_update(
        &program, astate, messages, coinputs, extra, vinput,
    )?;

    // Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

/// Applies the effects of an update, but does not check the messages.  It's
/// assumed we have a proof attesting to the validity that transitively attests
/// to this.
///
/// This is used in clients after they have a proof for an update to reconstruct
/// the actual state proven by the proof.
pub fn apply_update_operation_unconditionally(
    astate: &mut EeAccountState,
    operation: &UpdateInputData,
) -> EnvProgramResult<()> {
    // Parse extra data.
    let extra: UpdateExtraData =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;

    // Parse and process all messages using standalone functions.
    for (idx, entry) in operation.processed_messages().iter().enumerate() {
        let msg = parse_input_message(entry);
        ee_program::process_ee_message(astate, msg)?;
    }

    // Finalize state.
    ee_program::finalize_ee_state(astate, &extra)?;

    // Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

/// Applies the final changes to the account state.
///
/// This just updates the exec tip blkid and removes pending entries.
pub fn apply_final_update_changes(
    state: &mut EeAccountState,
    extra: &UpdateExtraData,
) -> EnvProgramResult<()> {
    // 1. Update final execution head block.
    state.set_last_exec_blkid(*extra.new_tip_blkid());

    // 2. Update queues.
    state.remove_pending_inputs(*extra.processed_inputs() as usize);
    state.remove_pending_fincls(*extra.processed_fincls() as usize);

    Ok(())
}

fn verify_acct_state_matches(
    _astate: &EeAccountState,
    _exp_new_state: &Hash,
) -> Result<(), EnvError> {
    // TODO use SSZ hash_tree_root
    Ok(())
}
