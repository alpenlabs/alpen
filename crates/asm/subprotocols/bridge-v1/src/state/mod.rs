//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::l1::BitcoinAmount;

use crate::{
    errors::{DepositError, WithdrawalValidationError},
    state::{assignment::AssignmentTable, deposit::DepositsTable, operator::OperatorTable},
    txs::{
        deposit::{DepositInfo, validate_deposit_output_lock, validate_drt_spending_signature},
        withdrawal::WithdrawalInfo,
    },
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

    amount: BitcoinAmount,
}

impl BridgeV1State {
    /// Creates a new bridge state with the specified amount and operator table.
    ///
    /// Initializes all component tables as empty and sets the expected deposit amount
    /// that will be used for deposit validation.
    ///
    /// # Parameters
    ///
    /// - `operators` - Table of registered bridge operators
    /// - `amount` - Expected deposit amount for validation
    ///
    /// # Returns
    ///
    /// A new [`BridgeV1State`] instance.
    pub fn new(operators: OperatorTable, amount: BitcoinAmount) -> Self {
        Self {
            operators,
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new_empty(),
            amount,
        }
    }

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

    /// Processes a parsed deposit by validating and adding it to the deposits table.
    ///
    /// This function takes already parsed deposit information, validates it against the
    /// current bridge state, and creates a new deposit entry in the deposits table if
    /// validation passes. Only operators that are part of the current N/N multisig set
    /// are included as notary operators for the deposit.
    ///
    /// # Parameters
    ///
    /// - `deposit_info` - Parsed deposit information containing amount, destination, and outpoint
    ///
    /// # Returns
    ///
    /// - `Ok(u32)` - The deposit index assigned to the newly created deposit entry
    /// - `Err(DepositParseError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount is zero or negative
    /// - The internal key doesn't match the current aggregated operator key
    /// - The deposit index already exists in the deposits table
    pub fn process_deposit(
        &mut self,
        tx: &Transaction,
        info: &DepositInfo,
    ) -> Result<u32, DepositError> {
        // Validate the deposit first
        self.validate_deposit(tx, info)?;

        let notary_operators = self.operators().current_multisig_indices().collect();
        let deposit_idx =
            self.deposits
                .create_next_deposit(info.outpoint, notary_operators, info.amt);

        Ok(deposit_idx)
    }

    /// Validates a deposit transaction and info against bridge state requirements.
    ///
    /// This function performs comprehensive validation of a deposit by verifying:
    /// - The deposit amount matches the bridge's expected amount
    /// - The Deposit Request Transaction (DRT) spending signature is valid
    /// - The deposit output is properly locked to the aggregated operator key
    /// - The deposit index is unique within the deposits table
    ///
    /// # Parameters
    ///
    /// - `tx` - The Bitcoin transaction containing the deposit
    /// - `info` - The parsed deposit information to validate
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the deposit passes all validation checks
    /// - `Err(DepositError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount doesn't match the bridge's expected amount
    /// - The DRT spending signature is invalid or doesn't match the aggregated operator key
    /// - The deposit output lock is incorrect
    /// - A deposit with the same index already exists
    fn validate_deposit(&self, tx: &Transaction, info: &DepositInfo) -> Result<(), DepositError> {
        // Verify the deposit amount matches the bridge's expected amount
        if info.amt.to_sat() != self.amount.to_sat() {
            return Err(DepositError::InvalidDepositAmount {
                expected: self.amount.to_sat(),
                actual: info.amt.to_sat(),
            });
        }

        // Validate the DRT spending signature against the aggregated operator key
        validate_drt_spending_signature(
            tx,
            info.drt_tapnode_hash,
            self.operators().agg_key(),
            info.amt.into(),
        )?;

        // Ensure the deposit output is properly locked to the aggregated operator key
        validate_deposit_output_lock(tx, self.operators().agg_key())?;

        // Verify this deposit index hasn't been used before
        if self.deposits().get_deposit(info.deposit_idx).is_some() {
            return Err(DepositError::DepositIdxAlreadyExists(info.deposit_idx));
        }

        Ok(())
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
        let deposit_idx = withdrawal_info.deposit_idx;

        // Check if an assignment exists for this deposit
        let assignment = self
            .assignments
            .get_assignment(deposit_idx)
            .ok_or(WithdrawalValidationError::NoAssignmentFound { deposit_idx })?;

        // Validate that the operator matches the assignment
        let expected_operator = assignment.assignee();
        let actual_operator = withdrawal_info.operator_idx;
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
        let actual_txid = withdrawal_info.deposit_txid;

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

        let actual_amount = withdrawal_info.withdrawal_amount;
        if expected_amount != actual_amount {
            return Err(WithdrawalValidationError::AmountMismatch {
                expected: expected_amount,
                actual: actual_amount,
            });
        }

        Ok(())
    }
}
