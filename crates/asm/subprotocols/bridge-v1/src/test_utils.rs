use std::any::Any;

use rand::Rng;
use strata_asm_common::{AsmCompactMmr, AsmLogEntry, AsmMmr, AuxData, InterprotoMsg, MsgRelayer, VerifiedAuxData};
use strata_asm_txs_bridge_v1::{
    deposit::{DepositInfo, parse_deposit_tx},
    deposit_request::DrtHeaderAux,
    test_utils::{create_connected_drt_and_dt, create_test_operators, parse_sps50_tx},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, WithdrawalFulfillmentTxHeaderAux},
};
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

/// Helper function to setup a complete deposit test scenario.
///
/// Creates a bridge state with test operators, generates connected DRT and DT transactions,
/// parses the deposit info, and prepares the verified auxiliary data needed for validation.
/// This consolidates the common setup logic used across deposit-related tests.
///
/// # Parameters
///
/// - `drt_aux` - The deposit request transaction header auxiliary data
///
/// # Returns
///
/// A tuple containing:
/// - `BridgeV1State` - The initialized bridge state with test operators
/// - `VerifiedAuxData` - The verified auxiliary data containing the DRT
/// - `DepositInfo` - The parsed deposit information from the deposit transaction
pub(crate) fn setup_deposit_test(
    drt_aux: &DrtHeaderAux,
) -> (BridgeV1State, VerifiedAuxData, DepositInfo) {
    // 1. Setup Bridge State
    let (state, operators) = create_test_state();

    // 2. Prepare DRT & DT
    let dt_aux = ArbitraryGenerator::new().generate();

    let (drt, dt) = create_connected_drt_and_dt(
        drt_aux,
        dt_aux,
        (*state.denomination()).into(),
        &operators,
    );

    // 3. Extract DepositInfo
    let dt_input = parse_sps50_tx(&dt);
    let info = parse_deposit_tx(&dt_input).expect("Should parse deposit tx");

    // 4. Prepare VerifiedAuxData containing the DRT
    let raw_drt: RawBitcoinTx = drt.clone().into();
    let aux_data = AuxData::new(vec![], vec![raw_drt]);
    let mmr = AsmMmr::new(16); // Dummy MMR, not used for tx lookup
    let compact_mmr: AsmCompactMmr = mmr.into();
    let verified_aux_data =
        VerifiedAuxData::try_new(&aux_data, &compact_mmr).expect("Should verify aux data");

    (state, verified_aux_data, info)
}
