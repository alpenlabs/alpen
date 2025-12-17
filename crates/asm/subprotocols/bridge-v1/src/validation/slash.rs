use strata_asm_common::VerifiedAuxData;
use strata_asm_txs_bridge_v1::slash::SlashInfo;

use crate::{errors::SlashValidationError, state::BridgeV1State};

/// Validates the parsed [`SlashInfo`].
///
/// The checks performed are:
/// 1. The stake connector is locked to one of the historical N/N multisig configurations.
///
/// Auxiliary data must provide the stake connector transaction output needed for this inspection.
pub(crate) fn validate_slash_info(
    state: &BridgeV1State,
    info: &SlashInfo,
    verified_aux_data: &VerifiedAuxData,
) -> Result<(), SlashValidationError> {
    let stake_connector_txout =
        verified_aux_data.get_bitcoin_txout(info.stake_inpoint().outpoint())?;
    let stake_connector_script = &stake_connector_txout.script_pubkey;

    if !state
        .operators()
        .historical_nn_scripts()
        .any(|script| script == stake_connector_script)
    {
        return Err(SlashValidationError::InvalidStakeConnectorScript);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        SlashValidationError,
        test_utils::{create_test_state, setup_slash_test},
        validation::validate_slash_info,
    };

    #[test]
    fn test_slash_tx_validation_success() {
        let (state, operators) = create_test_state();
        let (info, aux) = setup_slash_test(1, &operators);
        validate_slash_info(&state, &info, &aux).expect("handling valid slash info should succeed");
    }

    #[test]
    fn test_slash_tx_invalid_signers() {
        let (state, mut operators) = create_test_state();
        operators.pop();
        let (info, aux) = setup_slash_test(1, &operators);
        let err = validate_slash_info(&state, &info, &aux).unwrap_err();
        assert!(matches!(
            err,
            SlashValidationError::InvalidStakeConnectorScript
        ));
    }
}
