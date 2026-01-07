use std::any::Any;

use rand::Rng;
use strata_asm_common::{AsmLogEntry, AsmMmr, AuxData, InterprotoMsg, MsgRelayer, VerifiedAuxData};
use strata_asm_txs_bridge_v1::{
    deposit::{DepositInfo, parse_deposit_tx},
    deposit_request::DrtHeaderAux,
    slash::{SlashInfo, SlashTxHeaderAux, parse_slash_tx},
    test_utils::{
        create_connected_drt_and_dt, create_connected_stake_and_slash_txs,
        create_connected_stake_and_unstake_txs, create_test_operators, parse_sps50_tx,
    },
    unstake::{UnstakeInfo, UnstakeTxHeaderAux, parse_unstake_tx},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, WithdrawalFulfillmentTxHeaderAux},
};
use strata_bridge_types::OperatorIdx;
use strata_btc_types::RawBitcoinTx;
use strata_crypto::EvenSecretKey;
use strata_primitives::l1::{BitcoinAmount, L1BlockCommitment};
use strata_test_utils::ArbitraryGenerator;

use super::*;
use crate::state::{assignment::AssignmentEntry, config::BridgeV1Config};

/// A Mock MsgRelayer that does nothing.
///
/// This is used in tests where we don't care about the messages being emitted.
pub(crate) struct MockMsgRelayer;

impl MsgRelayer for MockMsgRelayer {
    fn relay_msg(&mut self, _m: &dyn InterprotoMsg) {}
    fn emit_log(&mut self, _log: AsmLogEntry) {}
    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Helper function to create a test bridge state and associated operator keys.
///
/// This function initializes a `BridgeV1State` with a randomly generated number of operators
/// (between 2 and 5), a fixed denomination, and an assignment duration. It returns the
/// initialized state along with the private keys of the operators, which can be used for
/// signing test transactions.
///
/// # Returns
///
/// - `(BridgeV1State, Vec<EvenSecretKey>)` - A tuple containing the initialized bridge state and a
///   vector of `EvenSecretKey` for the operators.
pub(crate) fn create_test_state() -> (BridgeV1State, Vec<EvenSecretKey>) {
    let mut rng = rand::thread_rng();
    let num_operators = rng.gen_range(2..=5);
    let (privkeys, operators) = create_test_operators(num_operators);
    let denomination = BitcoinAmount::from_sat(1_000_000);
    let config = BridgeV1Config {
        denomination,
        operators,
        assignment_duration: 144, // ~24 hours
        operator_fee: BitcoinAmount::from_sat(100_000),
    };
    let bridge_state = BridgeV1State::new(&config);
    (bridge_state, privkeys)
}

/// Helper function to add multiple test deposits to the bridge state.
///
/// Creates the specified number of deposits with randomly generated deposit info,
/// but ensures each deposit uses the bridge's expected denomination amount.
/// Each deposit is processed through the full validation pipeline.
///
/// # Parameters
///
/// - `state` - Mutable reference to the bridge state to add deposits to
/// - `count` - Number of deposits to create and add
/// - `privkeys` - Private keys used to sign the deposit transactions
pub(crate) fn add_deposits(state: &mut BridgeV1State, count: usize) -> Vec<DepositInfo> {
    let mut arb = ArbitraryGenerator::new();
    let mut infos = Vec::new();
    for _ in 0..count {
        let mut info: DepositInfo = arb.generate();
        info.set_amt(*state.denomination());
        state.add_deposit(&info).unwrap();
        infos.push(info);
    }
    infos
}

/// Helper function to add deposits and immediately create withdrawal assignments.
///
/// This is a convenience function that combines deposit creation with assignment
/// creation. For each deposit added, it creates a corresponding withdrawal command
/// and assignment. This simulates a complete deposit-to-assignment flow for testing.
///
/// # Parameters
///
/// - `state` - Mutable reference to the bridge state
/// - `count` - Number of deposit-assignment pairs to create
/// - `privkeys` - Private keys used to sign the deposit transactions
pub(crate) fn add_deposits_and_assignments(state: &mut BridgeV1State, count: usize) {
    add_deposits(state, count);
    let mut arb = ArbitraryGenerator::new();
    for _ in 0..count {
        let l1blk: L1BlockCommitment = arb.generate();
        let mut output: WithdrawOutput = arb.generate();
        output.amt = *state.denomination();
        state.create_withdrawal_assignment(&output, &l1blk).unwrap();
    }
}

/// Helper function to create withdrawal info that matches an existing assignment.
///
/// Extracts all the necessary information from an assignment entry to create
/// a WithdrawalInfo struct that would pass validation. This is used in tests
/// to create valid withdrawal fulfillment transactions.
///
/// # Parameters
///
/// - `assignment` - The assignment entry to extract information from
///
/// # Returns
///
/// A WithdrawalInfo struct with matching operator, deposit, and withdrawal details
pub(crate) fn create_withdrawal_info_from_assignment(
    assignment: &AssignmentEntry,
) -> WithdrawalFulfillmentInfo {
    let header_aux = WithdrawalFulfillmentTxHeaderAux::new(assignment.deposit_idx());
    WithdrawalFulfillmentInfo::new(
        header_aux,
        assignment.withdrawal_command().destination().to_script(),
        assignment.withdrawal_command().net_amount(),
    )
}

/// Helper function to create `VerifiedAuxData` from a list of Bitcoin transactions.
///
/// Creates a dummy MMR and wraps the provided transactions in `AuxData`, then
/// verifies it. This is useful for tests that need to provide auxiliary transaction
/// data for validation.
pub(crate) fn create_verified_aux_data(txs: Vec<RawBitcoinTx>) -> VerifiedAuxData {
    let aux_data = AuxData::new(vec![], txs);
    let mmr = AsmMmr::new(16); // Dummy MMR, not used for tx lookup in tests
    VerifiedAuxData::try_new(&aux_data, &mmr).expect("Should verify aux data")
}

/// Helper function to setup a complete deposit test scenario.
///
/// Generates connected DRT and DT transactions, parses the deposit info, and prepares
/// the verified auxiliary data needed for validation. This consolidates the common
/// setup logic used across deposit-related tests.
///
/// # Parameters
///
/// - `drt_aux` - The deposit request transaction header auxiliary data
/// - `denomination` - The bridge denomination to use for the deposit
/// - `operators` - The operator private keys for signing transactions
///
/// # Returns
///
/// A tuple containing:
/// - `VerifiedAuxData` - The verified auxiliary data containing the DRT
/// - `DepositInfo` - The parsed deposit information from the deposit transaction
pub(crate) fn setup_deposit_test(
    drt_aux: &DrtHeaderAux,
    denomination: BitcoinAmount,
    operators: &[EvenSecretKey],
) -> (VerifiedAuxData, DepositInfo) {
    // 1. Prepare DRT & DT
    let dt_aux = ArbitraryGenerator::new().generate();

    let (drt, dt) = create_connected_drt_and_dt(drt_aux, dt_aux, denomination.into(), operators);

    // 2. Extract DepositInfo
    let dt_input = parse_sps50_tx(&dt);
    let info = parse_deposit_tx(&dt_input).expect("Should parse deposit tx");

    // 3. Prepare VerifiedAuxData containing the DRT
    let raw_drt: RawBitcoinTx = drt.clone().into();
    let verified_aux_data = create_verified_aux_data(vec![raw_drt]);

    (verified_aux_data, info)
}

/// Helper function to setup a complete slash test scenario.
///
/// Generates connected stake and slash transactions, parses the slash info, and prepares
/// the verified auxiliary data needed for validation. This consolidates the common
/// setup logic used across slash-related tests.
///
/// # Parameters
///
/// - `operator_idx` - The index of the operator being slashed
/// - `operators` - The operator private keys for signing transactions
///
/// # Returns
///
/// A tuple containing:
/// - `SlashInfo` - The parsed slash information from the slash transaction
/// - `VerifiedAuxData` - The verified auxiliary data containing the stake transaction
pub(crate) fn setup_slash_test(
    operator_idx: OperatorIdx,
    operators: &[EvenSecretKey],
) -> (SlashInfo, VerifiedAuxData) {
    // 1. Prepare Slash Info and Transactions
    let slash_header = SlashTxHeaderAux::new(operator_idx);
    let (stake_tx, slash_tx) = create_connected_stake_and_slash_txs(&slash_header, operators);

    // 2. Prepare ParsedTx
    // We need to re-parse the slash tx to get the correct SlashInfo with updated input
    // (create_connected_stake_and_slash_txs updates the input to point to stake_tx)
    let slash_tx_input = parse_sps50_tx(&slash_tx);
    let parsed_slash_info = parse_slash_tx(&slash_tx_input).expect("Should parse slash tx");

    // 3. Prepare VerifiedAuxData containing the stake transaction
    let raw_stake_tx: RawBitcoinTx = stake_tx.clone().into();
    let verified_aux_data = create_verified_aux_data(vec![raw_stake_tx]);

    (parsed_slash_info, verified_aux_data)
}

/// Helper function to setup a complete unstake test scenario.
///
/// Generates connected stake and unstake transactions, parses the unstake info, and prepares
/// the verified auxiliary data needed for validation. This consolidates the common
/// setup logic used across unstake-related tests.
///
/// # Parameters
///
/// - `operator_idx` - The index of the operator being unstaked
/// - `operators` - The operator private keys for signing transactions
///
/// # Returns
///
/// A tuple containing:
/// - `UnstakeInfo` - The parsed unstake information from the unstake transaction
/// - `VerifiedAuxData` - The verified auxiliary data containing the stake transaction
pub(crate) fn setup_unstake_test(
    operator_idx: OperatorIdx,
    operators: &[EvenSecretKey],
) -> (UnstakeInfo, VerifiedAuxData) {
    // 1. Prepare Unstake Info and Transactions
    let unstake_header = UnstakeTxHeaderAux::new(operator_idx);
    let (stake_tx, unstake_tx) = create_connected_stake_and_unstake_txs(&unstake_header, operators);

    // 2. Prepare ParsedTx
    // We need to re-parse the unstake tx to get the correct UnstakeInfo with updated input
    // (create_connected_stake_and_unstake_txs updates the input to point to stake_tx)
    let unstake_tx_input = parse_sps50_tx(&unstake_tx);
    let parsed_unstake_info = parse_unstake_tx(&unstake_tx_input).expect("Should parse unstake tx");

    // 3. Prepare VerifiedAuxData containing the stake transaction
    let raw_stake_tx: RawBitcoinTx = stake_tx.clone().into();
    let verified_aux_data = create_verified_aux_data(vec![raw_stake_tx]);

    (parsed_unstake_info, verified_aux_data)
}
