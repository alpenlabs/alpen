use std::any::Any;

use rand::Rng;
use strata_asm_common::{AsmLogEntry, InterprotoMsg, MsgRelayer};
use strata_asm_txs_bridge_v1::{
    deposit::DepositInfo,
    test_utils::{create_test_deposit_tx, create_test_operators},
    withdrawal_fulfillment::{WithdrawalFulfillmentInfo, WithdrawalFulfillmentTxHeaderAux},
};
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
pub(crate) fn add_deposits(
    state: &mut BridgeV1State,
    count: usize,
    privkeys: &[EvenSecretKey],
) -> Vec<DepositInfo> {
    let mut arb = ArbitraryGenerator::new();
    let mut infos = Vec::new();
    for _ in 0..count {
        let mut info: DepositInfo = arb.generate();
        info.set_amt(*state.denomination());
        let tx = create_test_deposit_tx(&info, privkeys);
        state.process_deposit_tx(&tx, &info).unwrap();
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
pub(crate) fn add_deposits_and_assignments(
    state: &mut BridgeV1State,
    count: usize,
    privkeys: &[EvenSecretKey],
) {
    add_deposits(state, count, privkeys);
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
