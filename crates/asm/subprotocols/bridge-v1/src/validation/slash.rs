use bitcoin::ScriptBuf;

use crate::{errors::SlashValidationError, state::BridgeV1State};

/// Validates the stake connector script for a slash transaction locked to one of the historical N/N
/// multisig configurations.
pub(crate) fn validate_slash_stake_connector(
    state: &BridgeV1State,
    stake_connector_script: &ScriptBuf,
) -> Result<(), SlashValidationError> {
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
    use bitcoin::ScriptBuf;
    use strata_asm_common::VerifiedAuxData;
    use strata_asm_txs_bridge_v1::slash::SlashInfo;

    use crate::{
        SlashValidationError,
        test_utils::{create_test_state, setup_slash_test},
        validation::validate_slash_stake_connector,
    };

    fn stake_connector_script_from_aux(info: &SlashInfo, aux: &VerifiedAuxData) -> ScriptBuf {
        let txout = aux
            .get_bitcoin_txout(info.stake_inpoint().outpoint())
            .expect("stake connector txout should exist in aux data");
        txout.script_pubkey.clone()
    }

    #[test]
    fn test_slash_tx_validation_success() {
        let (state, operators) = create_test_state();
        let (info, aux) = setup_slash_test(1, &operators);
        let stake_connector_script = stake_connector_script_from_aux(&info, &aux);
        validate_slash_stake_connector(&state, &stake_connector_script)
            .expect("handling valid slash info should succeed");
    }

    #[test]
    fn test_slash_tx_invalid_signers() {
        let (state, mut operators) = create_test_state();
        operators.pop();
        let (info, aux) = setup_slash_test(1, &operators);
        let stake_connector_script = stake_connector_script_from_aux(&info, &aux);
        let err = validate_slash_stake_connector(&state, &stake_connector_script).unwrap_err();
        assert!(matches!(
            err,
            SlashValidationError::InvalidStakeConnectorScript
        ));
    }
}
