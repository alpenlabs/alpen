// TODO this is being refactored

use strata_ee_acct_types::{EnvError, ExecutionEnvironment};
use strata_snark_acct_runtime::{PrivateInput as UpdatePrivateInput, ProgramError, ProgramResult};

use crate::{EeSnarkAccountProgram, EeVerificationInput, private_input::PrivateInput};

pub fn verify_and_process_update<E: ExecutionEnvironment>(
    ee: &E,
    ee_priv_input: PrivateInput,
    upd_priv_input: UpdatePrivateInput,
) -> ProgramResult<(), EnvError> {
    // 1. Construct verification input.
    let vinput = EeVerificationInput::new(
        ee_priv_input.raw_prev_header(),
        ee_priv_input.raw_partial_pre_state(),
        ee,
    );

    // TODO something with the input chunks

    // 2. Construct the program instance and call out to the general update
    // processing.
    let prog = EeSnarkAccountProgram::<E>::new();
    strata_snark_acct_runtime::verify_and_process_update(&prog, &upd_priv_input, vinput)?;

    Ok(())
}
