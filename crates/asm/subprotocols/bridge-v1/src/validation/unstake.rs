use strata_asm_txs_bridge_v1::unstake::UnstakeInfo;
use strata_primitives::l1::BitcoinXOnlyPublicKey;

use crate::{
    errors::UnstakeValidationError,
    state::{BridgeV1State, operator::build_nn_script},
};

/// Validates the parsed [`UnstakeInfo`].
///
/// The checks performed are:
/// 1. The witness-pushed pubkey corresponds to a valid historical N/N pubkey.
///
/// Validation is performed by constructing a key-path-only P2TR script using [`build_nn_script`]
/// from the extracted pubkey and checking if it matches any historical operator set scripts stored
/// in state. Since we don't store historical pubkeys directly (only their P2TR representations),
/// we create a P2TR script from the extracted pubkey for comparison.
pub(crate) fn validate_unstake_info(
    state: &BridgeV1State,
    info: &UnstakeInfo,
) -> Result<(), UnstakeValidationError> {
    let witness_pubkey = BitcoinXOnlyPublicKey::from(*info.witness_pushed_pubkey());
    let expected_script = build_nn_script(&witness_pubkey);

    if !state
        .operators()
        .historical_nn_scripts()
        .any(|script| script == expected_script.inner())
    {
        return Err(UnstakeValidationError::InvalidStakeConnectorScript);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        UnstakeValidationError,
        test_utils::{create_test_state, setup_unstake_test},
        validation::validate_unstake_info,
    };

    #[test]
    fn test_unstake_tx_validation_success() {
        let (state, operators) = create_test_state();
        let (info, _aux) = setup_unstake_test(1, &operators);
        validate_unstake_info(&state, &info).expect("handling valid unstake info should succeed");
    }

    #[test]
    fn test_unstake_tx_invalid_signers() {
        let (state, mut operators) = create_test_state();
        operators.pop();
        let (info, _aux) = setup_unstake_test(1, &operators);
        let err = validate_unstake_info(&state, &info).unwrap_err();
        assert!(matches!(
            err,
            UnstakeValidationError::InvalidStakeConnectorScript
        ));
    }
}
