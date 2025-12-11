use bitcoin::{ScriptBuf, secp256k1::SECP256K1};
use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::NewExportEntry;
use strata_asm_txs_bridge_v1::parser::{ParsedDepositTx, ParsedTx};

use crate::{SlashValidationError, errors::BridgeSubprotocolError, state::BridgeV1State};

/// Handles parsed bridge transactions.
///
/// This function processes each transaction type according to its specific requirements:
/// - Validating transaction-specific rules and constraints
/// - Updating the bridge state
/// - Emitting logs or relaying InterProtocolMsg if needed
///
/// # Returns
/// * `Ok(())` if the transaction was processed successfully
/// * `Err(BridgeSubprotocolError)` if validation fails or an error occurred during processing
pub(crate) fn handle_parsed_tx<'t>(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(parsed_deposit_tx) => {
            let ParsedDepositTx { tx, info } = parsed_deposit_tx;
            state.process_deposit_tx(tx, &info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            let unlock = state.process_withdrawal_fulfillment_tx(&info)?;

            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log = NewExportEntry::new(container_id, unlock.compute_hash());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:PG"));

            Ok(())
        }
        ParsedTx::Slash(info) => {
            let stake_connector_script = &verified_aux_data
                .get_bitcoin_txout(info.second_inpoint().outpoint())?
                .script_pubkey;

            // Validate that the stake connector (second input) is locked to a known N/N multisig
            // script from any recorded configuration.
            if !state
                .operators()
                .historical_nn_scripts()
                .any(|script| script == stake_connector_script)
            {
                return Err(SlashValidationError::InvalidStakeConnectorScript.into());
            };

            // Remove the slashed operator
            state.remove_operator(info.header_aux().operator_idx());

            Ok(())
        }
        ParsedTx::Unstake(info) => {
            // Build P2TR scriptPubKey from the extracted pubkey. This needs to be validated against
            // known operator configurations.
            let extracted_pubkey_script =
                ScriptBuf::new_p2tr(SECP256K1, *info.witness_pushed_pubkey(), None);

            // Verify the extracted pubkey corresponds to a known operator configuration.
            if !state
                .operators()
                .historical_nn_scripts()
                .any(|script| script == &extracted_pubkey_script)
            {
                return Err(SlashValidationError::InvalidStakeConnectorScript.into());
            };

            state.remove_operator(info.header_aux().operator_idx());

            Ok(())
        }
    }
}

/// Pre-processes a parsed transaction to collect auxiliary data requests.
///
/// This function inspects the transaction type and requests any additional data needed
/// for the main processing phase.
pub(crate) fn preprocess_parsed_tx<'t>(
    parsed_tx: ParsedTx<'t>,
    _state: &BridgeV1State,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(_) => {}
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Slash(info) => {
            // Requests the Bitcoin transaction spent by the stake connector (second input). We need
            // this information to verify the stake connector is locked to a known N/N multisig.
            collector.request_bitcoin_tx(info.second_inpoint().0.txid);
        }
        ParsedTx::Unstake(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::{AsmCompactMmr, AsmMmr, AuxData, VerifiedAuxData};
    use strata_asm_txs_bridge_v1::{
        parser::ParsedTx,
        slash::{SlashTxHeaderAux, parse_slash_tx},
        test_utils::{
            build_connected_stake_and_unstake_txs, create_connected_stake_and_slash_txs,
            parse_sps50_tx,
        },
        unstake::{UnstakeTxHeaderAux, parse_unstake_tx},
    };
    use strata_btc_types::RawBitcoinTx;

    use super::handle_parsed_tx;
    use crate::test_utils::{MockMsgRelayer, create_test_state};

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
        let slash_tx_input = parse_sps50_tx(&slash_tx);
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
    fn test_handle_unstake_tx_success() {
        // 1. Setup Bridge State
        let (mut state, operators) = create_test_state();

        // 2. Prepare Slash Info and Transactions
        // We act as if the first operator (index 0) is being slashed.
        let operator_idx = 0;
        let unstake_header = UnstakeTxHeaderAux::new(operator_idx);

        let (stake_tx, unstake_tx) =
            build_connected_stake_and_unstake_txs(&unstake_header, &operators);

        // 3. Prepare ParsedTx
        // We need to re-parse the slash tx to get the correct SlashInfo with updated input
        // (create_connected_stake_and_slash_txs updates the input to point to stake_tx)
        let unstake_tx_input = parse_sps50_tx(&unstake_tx);
        let parsed_unstake_info =
            parse_unstake_tx(&unstake_tx_input).expect("Should parse slash tx");
        let parsed_tx = ParsedTx::Unstake(parsed_unstake_info);

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
}
