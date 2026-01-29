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
use strata_codec::decode_buf_exact;
use strata_ee_acct_types::{
    EeAccountState, EnvError, EnvProgramResult, ExecutionEnvironment, UpdateExtraData,
};
use strata_snark_acct_runtime::InputMessage;
use strata_snark_acct_types::{UpdateInputData, UpdateOperationData};

use crate::{
    ee_program::{self, EeSnarkAccountProgram},
    private_input::SharedPrivateInput,
    verification_state::EeVerificationInput,
};

/// Verify if an update operation is valid.  Accepts coinputs corresponding to
/// each message to privately attest validity before applying effects.
///
/// This accepts the update operation data as an argument.
// TODO refactor this to just call into the general snark account runtime
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
        .map(InputMessage::from_msg_entry);

    // Create program.
    let program = EeSnarkAccountProgram::new();

    // Create verification input containing EE and private data.
    let vinput = EeVerificationInput::new(shared_private, ee);

    // Delegate to generic implementation.
    strata_snark_acct_runtime::verify_and_apply_update(
        &program, astate, messages, coinputs, extra, vinput,
    )?;

    // Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_proof_state().inner_state())?;

    Ok(())
}

/// Applies the effects of an update, but does not check the messages.  It's
/// assumed we have a proof attesting to the validity that transitively attests
/// to this.
///
/// This is used in clients after they have a proof for an update to reconstruct
/// the actual state proven by the proof.
// TODO refactor this to just call into the general snark account runtime
pub fn apply_update_operation_unconditionally(
    astate: &mut EeAccountState,
    operation: &UpdateInputData,
) -> EnvProgramResult<()> {
    // Parse extra data.
    let extra: UpdateExtraData =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;

    // Parse and process all messages using standalone functions.
    for (_idx, entry) in operation.processed_messages().iter().enumerate() {
        let msg = InputMessage::from_msg_entry(entry);
        ee_program::process_ee_message(astate, msg)?;
    }

    // Finalize state.
    ee_program::finalize_ee_state(astate, &extra)?;

    // Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

fn verify_acct_state_matches(
    _astate: &EeAccountState,
    _exp_new_state: &Hash,
) -> Result<(), EnvError> {
    // TODO use SSZ hash_tree_root
    Ok(())
}
