// TODO this is being refactored

use strata_ee_acct_types::{EeAccountState, EnvError, ExecutionEnvironment};
use strata_snark_acct_runtime::{
    ArchivedPrivateInput as ArchivedUpdatePrivateInput, ProgramResult,
};
use strata_snark_acct_types::UpdateManifest;

use crate::{
    EeSnarkAccountProgram, EeVerificationInput,
    private_input::ArchivedPrivateInput as ArchivedEePrivateInput,
};

/// Verifies an update using various private inputs containing the account data
/// and an execution environment context.
pub fn verify_and_process_update<E: ExecutionEnvironment>(
    ee: &E,
    ee_priv_input: &ArchivedEePrivateInput,
    upd_priv_input: &ArchivedUpdatePrivateInput,
) -> ProgramResult<(), EnvError> {
    // 1. Construct verification input.
    let vinput = EeVerificationInput::new(
        ee,
        ee_priv_input.chunks(),
        ee_priv_input.raw_partial_pre_state(),
    );

    // 2. Construct the program instance and call out to the general update
    // processing.
    let prog = EeSnarkAccountProgram::<E>::new();
    strata_snark_acct_runtime::verify_and_process_update(&prog, upd_priv_input, vinput)?;

    Ok(())
}

/// Processes changes to an account's inner state using an update manifest,
/// presumably fetched from RPCs or L1.
pub fn process_update_unconditionally<E: ExecutionEnvironment>(
    state: &mut EeAccountState,
    update_manifest: &UpdateManifest,
) -> ProgramResult<(), EnvError> {
    // 1. Construct the program instance and call out to the general update
    // processing.
    let prog = EeSnarkAccountProgram::<E>::new();
    strata_snark_acct_runtime::apply_update_unconditionally(&prog, state, update_manifest)?;

    Ok(())
}
