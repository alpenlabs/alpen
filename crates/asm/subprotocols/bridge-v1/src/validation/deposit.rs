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

    let expected_script = state.operators().current_nn_script();
    if info.locked_script() != expected_script {
        return Err(DepositValidationError::WrongOutputLock(Mismatch {
            expected: expected_script.clone(),
            got: info.locked_script().clone(),
        }));
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

#[cfg(test)]
mod tests {
    use strata_asm_txs_bridge_v1::deposit_request::create_deposit_request_locking_script;
    use strata_btc_types::BitcoinAmount;
    use strata_primitives::constants::RECOVER_DELAY;
    use strata_test_utils::ArbitraryGenerator;

    use crate::{
        DepositValidationError,
        test_utils::{create_test_state, setup_deposit_test},
        validation::validate_deposit_info,
    };

    #[test]
    fn test_validate_deposit_tx_success() {
        let (state, operators) = create_test_state();
        let drt_aux = ArbitraryGenerator::new().generate();
        let (aux, info) = setup_deposit_test(&drt_aux, *state.denomination(), &operators);

        validate_deposit_info(&state, &info, &aux)
            .expect("handling valid deposit tx should succeed");
    }

    #[test]
    fn test_old_deposit_tx() {
        let (mut state, operators) = create_test_state();
        let drt_aux = ArbitraryGenerator::new().generate();
        let (aux, info) = setup_deposit_test(&drt_aux, *state.denomination(), &operators);

        let old_script = state.operators().current_nn_script().clone();
        state.remove_operator(1);
        let new_script = state.operators().current_nn_script().clone();

        let err = validate_deposit_info(&state, &info, &aux).unwrap_err();
        let DepositValidationError::WrongOutputLock(mismatch) = err else {
            panic!("Expected WrongOutputLock, got: {:?}", err);
        };

        assert_eq!(mismatch.expected, new_script);
        assert_eq!(mismatch.got, old_script);
    }

    #[test]
    fn test_old_signing_set() {
        let (mut state, operators) = create_test_state();
        let drt_aux = ArbitraryGenerator::new().generate();
        let (aux, mut info) = setup_deposit_test(&drt_aux, *state.denomination(), &operators);

        let old_agg_key = *state.operators().agg_key();
        state.remove_operator(1);
        let new_agg_key = state.operators().agg_key();

        // Set the correct locked script
        let locked_script = state.operators().current_nn_script().clone();
        info.set_locked_script(locked_script);

        let err = validate_deposit_info(&state, &info, &aux).unwrap_err();
        let DepositValidationError::DrtOutputScriptMismatch(mismatch) = err else {
            panic!("Expected DRTScriptMismatch, got: {:?}", err);
        };

        let new_valid_script = create_deposit_request_locking_script(
            drt_aux.recovery_pk(),
            new_agg_key.to_xonly_public_key(),
            RECOVER_DELAY,
        );
        let old_valid_script = create_deposit_request_locking_script(
            drt_aux.recovery_pk(),
            old_agg_key.to_xonly_public_key(),
            RECOVER_DELAY,
        );
        assert_eq!(mismatch.expected, new_valid_script);
        assert_eq!(mismatch.got, old_valid_script);
    }

    #[test]
    fn test_invalid_deposit_amount() {
        let (state, operators) = create_test_state();
        let drt_aux = ArbitraryGenerator::new().generate();
        let (aux, mut info) = setup_deposit_test(&drt_aux, *state.denomination(), &operators);

        let actual_amt = info.amt();
        let modified_amt: BitcoinAmount = ArbitraryGenerator::new().generate();
        info.set_amt(modified_amt);

        let err = validate_deposit_info(&state, &info, &aux).unwrap_err();
        let DepositValidationError::MismatchDepositAmount(mismatch) = err else {
            panic!("Expected MismatchDepositAmount, got: {:?}", err);
        };

        assert_eq!(mismatch.expected, actual_amt.to_sat());
        assert_eq!(mismatch.got, modified_amt.to_sat());
    }
}
