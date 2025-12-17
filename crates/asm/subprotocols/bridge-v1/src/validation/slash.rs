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
