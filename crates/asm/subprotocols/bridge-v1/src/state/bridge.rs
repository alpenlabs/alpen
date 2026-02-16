use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_bridge_msgs::WithdrawOutput;
use strata_asm_params::BridgeV1Config;
use strata_asm_txs_bridge_v1::{deposit::DepositInfo, errors::Mismatch};
use strata_bridge_types::OperatorIdx;
use strata_primitives::l1::{BitcoinAmount, L1BlockCommitment};

use crate::{
    errors::{DepositValidationError, WithdrawalCommandError},
    state::{
        assignment::{AssignmentEntry, AssignmentTable},
        deposit::{DepositEntry, DepositsTable},
        operator::OperatorTable,
        withdrawal::WithdrawalCommand,
    },
};

/// Main state container for the Bridge V1 subprotocol.
///
/// This structure holds all the persistent state for the bridge, including
/// operator registrations, deposit tracking, and assignment management.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    /// Table of registered bridge operators.
    operators: OperatorTable,

    /// Table of Bitcoin deposits managed by the bridge.
    deposits: DepositsTable,

    /// Table of operator assignments for withdrawal processing.
    assignments: AssignmentTable,

    /// The amount of bitcoin expected to be locked in the N/N multisig.
    denomination: BitcoinAmount,

    /// Amount the operator can take as fees for processing withdrawal.
    operator_fee: BitcoinAmount,

    /// Number of blocks after Deposit Request Transaction that the depositor can reclaim
    /// funds if operators fail to process the deposit.
    recovery_delay: u16,
}

impl BridgeV1State {
    /// Creates a new bridge state with the specified configuration.
    ///
    /// Initializes all component tables as empty, creates an operator table from the provided
    /// operator public keys, and sets the expected deposit denomination and deadline duration
    /// for validation and assignment management.
    ///
    /// # Parameters
    ///
    /// - `config` - Configuration containing operator public keys, denomination, and deadline
    ///   duration
    ///
    /// # Returns
    ///
    /// A new [`BridgeV1State`] instance.
    pub fn new(config: &BridgeV1Config) -> Self {
        let operators = OperatorTable::from_operator_list(&config.operators);
        Self {
            operators,
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new(config.assignment_duration),
            denomination: config.denomination,
            operator_fee: config.operator_fee,
            recovery_delay: config.recovery_delay,
        }
    }

    /// Returns a reference to the operator table.
    pub fn operators(&self) -> &OperatorTable {
        &self.operators
    }

    /// Returns a reference to the deposits table.
    pub fn deposits(&self) -> &DepositsTable {
        &self.deposits
    }

    /// Returns a reference to the assignments table.
    pub fn assignments(&self) -> &AssignmentTable {
        &self.assignments
    }

    /// Returns the deposit denomination.
    pub fn denomination(&self) -> &BitcoinAmount {
        &self.denomination
    }

    /// Returns the recovery delay to reclaim funds.
    pub fn recovery_delay(&self) -> u16 {
        self.recovery_delay
    }

    /// Processes a deposit transaction by validating and adding it to the deposits table.
    ///
    /// This function takes already parsed deposit transaction information, validates it against the
    /// current state, and creates a new deposit entry in the deposits table if
    /// validation passes. Only operators that are currently active in the N/N multisig set
    /// are included as notary operators for the deposit.
    ///
    /// # Parameters
    ///
    /// - `tx` - The deposit transaction
    /// - `info` - Parsed deposit information containing amount, destination, and outpoint
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the deposit is validated and inserted successfully
    /// - `Err(DepositValidationError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount is zero or negative
    /// - The internal key doesn't match the current aggregated operator key
    /// - The deposit index already exists in the deposits table
    pub fn add_deposit(&mut self, info: &DepositInfo) -> Result<(), DepositValidationError> {
        let notary_operators = self.operators.current_multisig().clone();
        let entry = DepositEntry::new(
            info.header_aux().deposit_idx(),
            notary_operators,
            info.amt(),
        )?;
        self.deposits.insert_deposit(entry)?;

        Ok(())
    }

    /// Adds a new withdrawal assignment to the assignments table.
    ///
    /// This retrieves the oldest unassigned deposit UTXO, validates that its amount matches
    /// the withdrawal amount, and creates a withdrawal command with the configured operator fee.
    /// The assignment is then added to the table with operators randomly selected from the
    /// currently active operators.
    ///
    /// # Parameters
    ///
    /// - `withdrawal_output` - The withdrawal output specifying destination and amount
    /// - `l1_block` - The L1 block commitment used for operator selection and deadline calculation
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal assignment was successfully added
    /// - `Err(WithdrawalCommandError)` - If no unassigned deposits, amounts mismatch, or adding new
    ///   assignment fails
    pub fn create_withdrawal_assignment(
        &mut self,
        withdrawal_output: &WithdrawOutput,
        l1_block: &L1BlockCommitment,
    ) -> Result<(), WithdrawalCommandError> {
        // Get the oldest deposit
        let deposit = self
            .deposits
            .remove_oldest_deposit()
            .ok_or(WithdrawalCommandError::NoUnassignedDeposits)?;

        if deposit.amt() != withdrawal_output.amt() {
            return Err(WithdrawalCommandError::DepositWithdrawalAmountMismatch(
                Mismatch {
                    expected: deposit.amt().to_sat(),
                    got: withdrawal_output.amt().to_sat(),
                },
            ));
        }

        let withdrawal_cmd = WithdrawalCommand::new(withdrawal_output.clone(), self.operator_fee);

        self.assignments.add_new_assignment(
            deposit,
            withdrawal_cmd,
            self.operators.current_multisig(),
            l1_block,
        )
    }

    /// Processes all expired assignments by reassigning them to new operators.
    ///
    /// This function iterates through all assignments, identifies those that have expired
    /// based on the current Bitcoin block height, and attempts to reassign them to new
    /// operators that haven't been previously assigned to the same withdrawal.
    ///
    /// # Parameters
    ///
    /// - `current_block` - The current L1 block commitment containing height and block hash
    ///
    /// # Returns
    ///
    /// - `Ok(Vec<u32>)` - Vector of deposit indices that were successfully reassigned
    /// - `Err(WithdrawalCommandError)` - If any reassignment fails
    ///
    /// # Notes
    ///
    /// If a reassignment fails for any expired assignment (e.g., no eligible operators
    /// remaining), the function returns an error and stops processing. Successfully
    /// reassigned deposits before the error are returned in the error context if needed.
    pub fn reassign_expired_assignments(
        &mut self,
        current_block: &L1BlockCommitment,
    ) -> Result<Vec<u32>, WithdrawalCommandError> {
        self.assignments
            .reassign_expired_assignments(self.operators.current_multisig(), current_block)
    }

    /// Removes an assignment by its deposit index.
    ///
    /// # Returns
    ///
    /// - `Some(AssignmentEntry)` if the assignment was found and removed
    /// - `None` if no assignment with the given deposit index exists
    pub fn remove_assignment(&mut self, deposit_idx: u32) -> Option<AssignmentEntry> {
        self.assignments.remove_assignment(deposit_idx)
    }

    /// Removes an operator from the active multisig by deactivating them.
    ///
    /// # Panics
    ///
    /// Panics if removing this operator would result in no active operators remaining.
    pub fn remove_operator(&mut self, operator_idx: OperatorIdx) {
        self.operators
            .apply_membership_changes(&[], &[operator_idx]);
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::l1::L1BlockCommitment;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{add_deposits, create_test_state};

    /// Test successful withdrawal assignment creation.
    ///
    /// Verifies that withdrawal assignments are created correctly by consuming
    /// unassigned deposits and creating assignment entries. Tests the progression
    /// from multiple deposits to assignments until no deposits remain.
    #[test]
    fn test_create_withdrawal_assignment_success() {
        let (mut state, _privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 4;
        add_deposits(&mut state, count);

        for i in 0..count {
            let unassigned_deposit_count = state.deposits.len();
            let assigned_deposit_count = state.assignments.len();
            assert_eq!(unassigned_deposit_count as usize, count - i);
            assert_eq!(assigned_deposit_count as usize, i);

            let l1blk: L1BlockCommitment = arb.generate();
            let mut output: WithdrawOutput = arb.generate();
            output.amt = state.denomination;
            let res = state.create_withdrawal_assignment(&output, &l1blk);
            assert!(res.is_ok());

            let unassigned_deposit_count = state.deposits.len();
            let assigned_deposit_count = state.assignments.len();
            assert_eq!(unassigned_deposit_count as usize, count - i - 1);
            assert_eq!(assigned_deposit_count as usize, i + 1);
        }

        let l1blk: L1BlockCommitment = arb.generate();
        let output: WithdrawOutput = arb.generate();
        let res = state.create_withdrawal_assignment(&output, &l1blk);
        assert!(res.is_err());
    }

    /// Test withdrawal assignment creation failure scenarios.
    ///
    /// Verifies that withdrawal assignment creation fails when there's a mismatch
    /// between the deposit amount and withdrawal command amount.
    #[test]
    fn test_create_withdrawal_assignment_failure() {
        let (mut state, _privkeys) = create_test_state();
        let mut arb = ArbitraryGenerator::new();

        let count = 1;
        let deposit = add_deposits(&mut state, count)[0].clone();

        let l1blk: L1BlockCommitment = arb.generate();
        let output: WithdrawOutput = arb.generate();
        let err = state
            .create_withdrawal_assignment(&output, &l1blk)
            .unwrap_err();
        assert!(matches!(
            err,
            WithdrawalCommandError::DepositWithdrawalAmountMismatch(..)
        ));
        if let WithdrawalCommandError::DepositWithdrawalAmountMismatch(mismatch) = err {
            assert_eq!(mismatch.got, output.amt.to_sat());
            assert_eq!(mismatch.expected, deposit.amt().to_sat());
        }
    }
}
