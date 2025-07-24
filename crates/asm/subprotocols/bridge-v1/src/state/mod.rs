//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::l1::BitcoinAmount;

use crate::{
    errors::WithdrawalValidationError,
    state::{assignment::AssignmentTable, deposit::DepositsTable, operator::OperatorTable},
    txs::{deposit::DepositInfo, withdrawal::WithdrawalInfo},
};

pub mod assignment;
pub mod deposit;
pub mod deposit_state;
pub mod operator;
pub mod withdrawal;

/// Main state container for the Bridge V1 subprotocol.
///
/// This structure holds all the persistent state for the bridge, including
/// operator registrations, deposit tracking, and assignment management.
///
/// # Fields
///
/// - `operators` - Table of registered bridge operators with their public keys
/// - `deposits` - Table of Bitcoin deposits with UTXO references and amounts
/// - `assignments` - Table linking deposits to operators with execution deadlines
///
/// # Serialization
///
/// The state is serializable using Borsh for efficient storage and transmission
/// within the Anchor State Machine.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1State {
    /// Table of registered bridge operators.
    operators: OperatorTable,

    /// Table of Bitcoin deposits managed by the bridge.
    deposits: DepositsTable,

    /// Table of operator assignments for withdrawal processing.
    assignments: AssignmentTable,
}

impl BridgeV1State {

    /// Returns a reference to the operator table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`OperatorTable`].
    pub fn operators(&self) -> &OperatorTable {
        &self.operators
    }

    /// Returns a mutable reference to the operator table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`OperatorTable`].
    pub fn operators_mut(&mut self) -> &mut OperatorTable {
        &mut self.operators
    }

    /// Returns a reference to the deposits table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`DepositsTable`].
    pub fn deposits(&self) -> &DepositsTable {
        &self.deposits
    }

    /// Returns a mutable reference to the deposits table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`DepositsTable`].
    pub fn deposits_mut(&mut self) -> &mut DepositsTable {
        &mut self.deposits
    }

    /// Returns a reference to the assignments table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`AssignmentTable`].
    pub fn assignments(&self) -> &AssignmentTable {
        &self.assignments
    }

    /// Returns a mutable reference to the assignments table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`AssignmentTable`].
    pub fn assignments_mut(&mut self) -> &mut AssignmentTable {
        &mut self.assignments
    }

    /// Processes a parsed deposit by adding it to the deposits table.
    ///
    /// This function takes already parsed deposit information and creates a new deposit entry
    /// in the deposits table. Only operators that are part of the current N/N multisig set
    /// are included as notary operators for the deposit.
    ///
    /// # Parameters
    ///
    /// - `deposit_info` - Parsed deposit information containing amount, destination, and outpoint
    ///
    /// # Returns
    ///
    /// The deposit index assigned to the newly created deposit entry.
    pub fn process_deposit(&mut self, deposit_info: &DepositInfo) -> u32 {
        let notary_operators = self.operators().current_multisig_indices().collect();
        self.deposits
            .create_next_deposit(deposit_info.outpoint, notary_operators, deposit_info.amt)
    }

    /// Processes a parsed withdrawal by validating it against assignment information.
    ///
    /// This function takes already parsed withdrawal information and validates it
    /// against the corresponding assignment entry. It checks that:
    /// - An assignment exists for the withdrawal's deposit
    /// - The operator performing the withdrawal matches the assigned operator
    /// - The withdrawal amounts and destinations match the assignment specifications
    ///
    /// # Parameters
    ///
    /// - `withdrawal_info` - Parsed withdrawal information containing operator, deposit details,
    ///   and amounts
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal is valid according to assignment information
    /// - `Err(WithdrawalValidationError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No assignment exists for the referenced deposit
    /// - The operator doesn't match the assigned operator
    /// - The withdrawal specifications don't match the assignment
    pub fn process_withdrawal(
        &self,
        withdrawal_info: &WithdrawalInfo,
    ) -> Result<(), WithdrawalValidationError> {
        let deposit_idx = withdrawal_info.deposit_idx();

        // Check if an assignment exists for this deposit
        let assignment = self
            .assignments
            .get_assignment(deposit_idx)
            .ok_or(WithdrawalValidationError::NoAssignmentFound { deposit_idx })?;

        // Validate that the operator matches the assignment
        let expected_operator = assignment.assignee();
        let actual_operator = withdrawal_info.operator_idx();
        if expected_operator != actual_operator {
            return Err(WithdrawalValidationError::OperatorMismatch {
                expected: expected_operator,
                actual: actual_operator,
            });
        }

        // Validate that the deposit txid matches
        let deposit = self
            .deposits
            .get_deposit(deposit_idx)
            .ok_or(WithdrawalValidationError::DepositNotFound { deposit_idx })?;

        let expected_txid = deposit.output().outpoint().txid;
        let actual_txid = withdrawal_info.deposit_txid();
        if expected_txid != actual_txid {
            return Err(WithdrawalValidationError::DepositTxidMismatch {
                expected: expected_txid,
                actual: actual_txid,
            });
        }

        // Validate withdrawal amount against assignment command
        let expected_amount: BitcoinAmount = assignment
            .withdrawal_command()
            .withdraw_outputs()
            .iter()
            .map(|output| output.amt())
            .sum();

        let actual_amount = withdrawal_info.withdrawal_amount();
        if expected_amount != actual_amount {
            return Err(WithdrawalValidationError::AmountMismatch {
                expected: expected_amount,
                actual: actual_amount,
            });
        }

        Ok(())
    }
}

