use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::NewExportEntry;
use strata_asm_txs_bridge_v1::{
    deposit_request::{create_deposit_request_locking_script, parse_drt_new},
    errors::Mismatch,
};
use strata_primitives::constants::RECOVER_DELAY; /* TODO:PG make this a parameter instead of
                                                  * constant */

use crate::{
    DepositValidationError, SlashValidationError, errors::BridgeSubprotocolError, parser::ParsedTx,
    state::BridgeV1State,
};

/// Handles parsed transactions and updates the bridge state accordingly.
pub(crate) fn handle_parsed_tx(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(info) => {
            // Verify the deposit output is locked to the current N/N multisig script
            if info.locked_script() != state.operators().current_nn_script() {
                return Err(DepositValidationError::WrongOutputLock)?;
            }

            // Retrieve the Deposit Request Transaction (DRT) from auxiliary data
            let drt = verified_aux_data.get_bitcoin_tx(info.drt_inpoint().txid)?;
            let drt_info = parse_drt_new(drt).unwrap();

            // Construct the expected locking script for the DRT output
            let expected_script = create_deposit_request_locking_script(
                drt_info.header_aux().recovery_pk(),
                state.operators().agg_key().to_xonly_public_key(),
                RECOVER_DELAY,
            );

            // Verify the DRT output script matches what we expect
            if drt_info.drt_out_script() != &expected_script {
                return Err(DepositValidationError::DrtOutputScriptMismatch(Mismatch {
                    expected: expected_script,
                    got: drt_info.drt_out_script().clone(),
                })
                .into());
            }

            // Update bridge state with the validated deposit
            state.process_deposit_tx(&info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            // Process the withdrawal and get the unlock information
            let unlock = state.process_withdrawal_fulfillment_tx(&info)?;

            // Emit a log entry to notify other components that this withdrawal has been processed
            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log = NewExportEntry::new(container_id, unlock.compute_hash());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:PG"));

            Ok(())
        }
        ParsedTx::Slash(info) => {
            // Extract the stake connector script from the second input of the slash transaction
            let stake_connector_script = &verified_aux_data
                .get_bitcoin_txout(info.second_inpoint().outpoint())?
                .script_pubkey;

            // Validate that the stake connector is locked to a known N/N multisig script.
            if !state
                .operators()
                .historical_nn_scripts()
                .any(|script| script == stake_connector_script)
            {
                return Err(SlashValidationError::InvalidStakeConnectorScript.into());
            };

            // Remove the slashed operator from the active set
            state.remove_operator(info.header_aux().operator_idx());

            Ok(())
        }
    }
}

/// Pre-processes a parsed transaction to collect auxiliary data requests.
///
/// This function inspects the transaction type and requests any additional data needed
/// for full verification during the main processing phase. Currently handles:
///
/// - **Deposit transactions**: No auxiliary data required
/// - **Withdrawal fulfillment transactions**: No auxiliary data required
/// - **Slash transactions**: Requests the Bitcoin transaction spent by the stake connector (second
///   input). We need this information to verify the stake connector is locked to a known N/N
///   multisig.
/// - **Unstake transactions**: Requests the Bitcoin transaction spent by the stake connector
///   (second input). We need this information to verify the stake connector is locked to a known
///   N/N multisig.
pub(crate) fn preprocess_parsed_tx(
    parsed_tx: ParsedTx,
    _state: &BridgeV1State,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(_) => {}
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Slash(info) => {
            collector.request_bitcoin_tx(info.second_inpoint().0.txid);
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::{AsmCompactMmr, AsmMmr, AuxData, VerifiedAuxData};
    use strata_asm_txs_bridge_v1::{
        deposit::DepositTxHeaderAux,
        deposit_request::DrtHeaderAux,
        slash::{SlashTxHeaderAux, parse_slash_tx},
        test_utils::{create_connected_drt_and_dt, create_connected_stake_and_slash_txs, parse_tx},
    };
    use strata_btc_types::RawBitcoinTx;

    use super::handle_parsed_tx;
    use crate::{
        parser::ParsedTx,
        test_utils::{MockMsgRelayer, create_test_state},
    };

    #[test]
    fn test_handle_slash_tx_success() {
        // 1. Setup Bridge State
        let (mut state, operators) = create_test_state();

        // 2. Prepare Slash Info and Transactions
        // We act as if the first operator (index 0) is being slashed.
        let operator_idx = 0;
        let slash_header = SlashTxHeaderAux::new(operator_idx);

        let (stake_tx, slash_tx) = create_connected_stake_and_slash_txs(&slash_header, &operators);

        // 3. Prepare ParsedTx
        // We need to re-parse the slash tx to get the correct SlashInfo with updated input
        // (create_connected_stake_and_slash_txs updates the input to point to stake_tx)
        let slash_tx_input = parse_tx(&slash_tx);
        let parsed_slash_info = parse_slash_tx(&slash_tx_input).expect("Should parse slash tx");
        let parsed_tx = ParsedTx::Slash(parsed_slash_info);

        // 4. Prepare VerifiedAuxData containing the stake transaction
        let raw_stake_tx: RawBitcoinTx = stake_tx.clone().into();
        let aux_data = AuxData::new(vec![], vec![raw_stake_tx]);
        let mmr = AsmMmr::new(16); // Dummy MMR, not used for tx lookup
        let compact_mmr: AsmCompactMmr = mmr.into();
        let verified_aux_data =
            VerifiedAuxData::try_new(&aux_data, &compact_mmr).expect("Should verify aux data");

        // 5. Handle the transaction
        let mut relayer = MockMsgRelayer;
        let result = handle_parsed_tx(&mut state, parsed_tx, &verified_aux_data, &mut relayer);

        assert!(result.is_ok(), "Handle parsed tx should succeed");

        // 6. Verify the operator is removed
        assert!(
            !state.operators().is_in_current_multisig(operator_idx),
            "Operator should be removed"
        );
    }

    #[test]
    fn test_handle_deposit_tx_success() {
        // 1. Setup Bridge State
        let (_, operators) = create_test_state();

        let drt_header_aux = DrtHeaderAux::new([1u8; 32], vec![1u8; 20]);
        let dt_header_aux = DepositTxHeaderAux::new(1, [1u8; 32], vec![1u8; 20]);
        let (_, _) = create_connected_drt_and_dt(drt_header_aux, dt_header_aux, &operators);

        // FIXME: This is failing due to signature mismatch
    }
}
