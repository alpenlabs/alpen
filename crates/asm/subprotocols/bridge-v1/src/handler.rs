use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::{DepositLog, NewExportEntry};
use strata_asm_txs_bridge_v1::{BRIDGE_V1_SUBPROTOCOL_ID, parser::ParsedTx};

use crate::{
    errors::BridgeSubprotocolError,
    state::{BridgeV1State, OperatorClaimUnlock},
    validation::{
        validate_deposit_info, validate_slash_info, validate_unstake_info,
        validate_withdrawal_fulfillment_info,
    },
};

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
pub(crate) fn handle_parsed_tx(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(info) => {
            validate_deposit_info(state, &info, verified_aux_data)?;
            state.add_deposit(&info)?;

            let deposit_log =
                DepositLog::new(0, info.amt().to_sat(), *info.header_aux().ee_address());
            relayer
                .emit_log(AsmLogEntry::from_log(&deposit_log).expect("deposit log must not fail"));

            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            validate_withdrawal_fulfillment_info(state, &info)?;
            let deposit_idx = info.header_aux().deposit_idx();

            let fulfilled_assignment = state
                .remove_assignment(deposit_idx)
                .expect("validation checks that the assignment exists");

            let unlock =
                OperatorClaimUnlock::new(deposit_idx, fulfilled_assignment.current_assignee());

            // Use SubprotocolId as the containerId.
            let withdrawal_processed_log =
                NewExportEntry::new(BRIDGE_V1_SUBPROTOCOL_ID, unlock.compute_hash());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:PG"));

            Ok(())
        }
        ParsedTx::Slash(info) => {
            validate_slash_info(state, &info, verified_aux_data)?;
            state.remove_operator(info.header_aux().operator_idx());
            Ok(())
        }
        ParsedTx::Unstake(info) => {
            validate_unstake_info(state, &info)?;
            state.remove_operator(info.header_aux().operator_idx());
            Ok(())
        }
    }
}

/// Pre-processes a parsed transaction to collect auxiliary data requests.
///
/// This function inspects the transaction type and requests any additional data needed
/// for the main processing phase.
pub(crate) fn preprocess_parsed_tx(
    parsed_tx: ParsedTx,
    _state: &BridgeV1State,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(info) => {
            // Request the Deposit Request Transaction (DRT) as auxiliary data.
            // We need this to verify the deposit chain and validate the DRT output locking script
            // during the main processing phase.
            dbg!("requesting tx");
            dbg!(&info.drt_inpoint().txid);
            collector.request_bitcoin_tx(info.drt_inpoint().txid);
        }
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Slash(info) => {
            // Requests the Bitcoin transaction spent by the stake connector (second input). We need
            // this information to verify the stake connector is locked to a known N/N multisig.
            collector.request_bitcoin_tx(info.stake_inpoint().0.txid);
        }
        ParsedTx::Unstake(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_txs_bridge_v1::{
        deposit_request::DrtHeaderAux,
        parser::ParsedTx,
        test_utils::{create_test_withdrawal_fulfillment_tx, parse_sps50_tx},
        withdrawal_fulfillment::parse_withdrawal_fulfillment_tx,
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::handle_parsed_tx;
    use crate::test_utils::{
        MockMsgRelayer, add_deposits_and_assignments, create_test_state, create_verified_aux_data,
        create_withdrawal_info_from_assignment, setup_slash_test, setup_unstake_test,
    };

    #[test]
    fn test_handle_deposit_tx_success() {
        // 1. Setup deposit test scenario
        let (mut state, operators) = create_test_state();
        let drt_aux: DrtHeaderAux = ArbitraryGenerator::new().generate();
        let (verified_aux_data, info) =
            crate::test_utils::setup_deposit_test(&drt_aux, *state.denomination(), &operators);

        // 2. Prepare ParsedTx
        let parsed_tx = ParsedTx::Deposit(info.clone());
        let deposit_idx = info.header_aux().deposit_idx();

        // 3. Deposits table doesn't have the deposit entry
        assert!(
            state.deposits().get_deposit(deposit_idx).is_none(),
            "entry should not exist"
        );

        // 4. Handle the transaction
        let mut relayer = MockMsgRelayer;
        handle_parsed_tx(&mut state, parsed_tx, &verified_aux_data, &mut relayer)
            .expect("handling valid deposit tx should succeed");

        // 5. Should add a new entry in the deposits table
        assert!(
            state.deposits().get_deposit(deposit_idx).is_some(),
            "entry should be added"
        );
    }

    #[test]
    fn test_handle_withdrawal_fulfillment_tx_success() {
        // 1. Setup Bridge State with deposits and assignments
        let (mut state, _) = create_test_state();

        let count = 3;
        add_deposits_and_assignments(&mut state, count);

        for _ in 0..count {
            let assignment = state.assignments().assignments().first().unwrap().clone();

            // 2. Prepare ParsedTx
            let withdrawal_info = create_withdrawal_info_from_assignment(&assignment);
            let tx = create_test_withdrawal_fulfillment_tx(&withdrawal_info);
            let tx_input = parse_sps50_tx(&tx);
            let parsed_info = parse_withdrawal_fulfillment_tx(&tx_input)
                .expect("should parse wthdrawal fulfillment tx");
            let parsed_tx = ParsedTx::WithdrawalFulfillment(parsed_info);

            let aux = create_verified_aux_data(vec![]);

            assert!(
                state
                    .assignments()
                    .get_assignment(assignment.deposit_idx())
                    .is_some(),
                "should have assignment before fulfillment"
            );

            // 3. Handle the transaction
            let mut relayer = MockMsgRelayer;
            handle_parsed_tx(&mut state, parsed_tx, &aux, &mut relayer)
                .expect("handling deposit tx should success");

            assert!(
                state
                    .assignments()
                    .get_assignment(assignment.deposit_idx())
                    .is_none(),
                "assignment should be removed after fulfillment"
            );
        }
    }

    #[test]
    fn test_handle_slash_tx_success() {
        let operator_idx = 1;
        let (mut state, operators) = create_test_state();
        let (info, aux) = setup_slash_test(operator_idx, &operators);

        assert!(
            state.operators().is_in_current_multisig(operator_idx),
            "Operator should be removed"
        );

        // 5. Handle the transaction
        let parsed_tx = ParsedTx::Slash(info);
        let mut relayer = MockMsgRelayer;
        let result = handle_parsed_tx(&mut state, parsed_tx, &aux, &mut relayer);

        assert!(result.is_ok(), "Handle parsed tx should succeed");

        // 6. Verify the operator is removed
        assert!(
            !state.operators().is_in_current_multisig(operator_idx),
            "Operator should be removed"
        );
    }

    #[test]
    fn test_handle_unstake_tx_success() {
        let operator_idx = 0;
        let (mut state, operators) = create_test_state();
        let (info, aux) = setup_unstake_test(operator_idx, &operators);

        assert!(
            state.operators().is_in_current_multisig(operator_idx),
            "Operator should be in current multisig"
        );

        // Handle the transaction
        let parsed_tx = ParsedTx::Unstake(info);
        let mut relayer = MockMsgRelayer;
        let result = handle_parsed_tx(&mut state, parsed_tx, &aux, &mut relayer);

        assert!(result.is_ok(), "Handle parsed tx should succeed");

        // Verify the operator is removed
        assert!(
            !state.operators().is_in_current_multisig(operator_idx),
            "Operator should be removed"
        );
    }
}
