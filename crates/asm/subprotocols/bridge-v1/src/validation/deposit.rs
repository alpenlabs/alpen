use strata_asm_common::VerifiedAuxData;
use strata_asm_txs_bridge_v1::{
    deposit::DepositInfo,
    deposit_request::{create_deposit_request_locking_script, parse_drt},
    errors::Mismatch,
};
use strata_primitives::constants::RECOVER_DELAY;

use crate::{errors::DepositValidationError, state::BridgeV1State};

/// Validates the parsed [`DepositInfo`].
///
/// The checks performed are:
/// 1. The deposit output is locked to the current aggregated N/N multisig script.
/// 2. The associated Deposit Request Transaction (DRT) output script matches the expected lock
///    script derived from the bridge configuration.
/// 3. The deposit amount equals the bridge’s configured denomination.
///
/// Auxiliary data must provide the DRT needed for these inspections.
pub(crate) fn validate_deposit_info(
    state: &BridgeV1State,
    info: &DepositInfo,
    verified_aux_data: &VerifiedAuxData,
) -> Result<(), DepositValidationError> {
    let drt_tx = verified_aux_data.get_bitcoin_tx(info.drt_inpoint().txid)?;
    let drt_info = parse_drt(drt_tx).unwrap();

    if info.locked_script() != state.operators().current_nn_script() {
        return Err(DepositValidationError::WrongOutputLock);
    }

    let expected_drt_script = create_deposit_request_locking_script(
        drt_info.header_aux().recovery_pk(),
        state.operators().agg_key().to_xonly_public_key(),
        RECOVER_DELAY,
    );
    let actual_script = &drt_info.deposit_request_output().inner().script_pubkey;

    if actual_script != &expected_drt_script {
        return Err(DepositValidationError::DrtOutputScriptMismatch(Mismatch {
            expected: expected_drt_script,
            got: actual_script.clone(),
        }));
    }

    let expected_amount = state.denomination().to_sat();
    if info.amt().to_sat() != expected_amount {
        return Err(DepositValidationError::MismatchDepositAmount(Mismatch {
            expected: expected_amount,
            got: info.amt().to_sat(),
        }));
    }

    Ok(())
}
