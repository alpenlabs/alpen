//! Bridge state types.
//!
//! This just implements a very simple n-of-n multisig bridge.  It will be
//! extended to a more sophisticated design when we have that specced out.

use bitcoin::Transaction;
use borsh::{BorshDeserialize, BorshSerialize};
// Re-export types that are needed in genesis config
pub use operator::OperatorTable;
use rand_chacha::{
    ChaChaRng,
    rand_core::{RngCore, SeedableRng},
};
use strata_primitives::{buf::Buf32, l1::BitcoinAmount, operator::OperatorPubkeys};

use crate::{
    errors::{DepositError, WithdrawalCommandError, WithdrawalValidationError},
    state::{
        assignment::AssignmentTable,
        deposit::DepositsTable,
        withdrawal::{WithdrawalCommand, WithdrawalProcessedInfo},
    },
    txs::{
        deposit::{DepositInfo, validate_deposit_output_lock, validate_drt_spending_signature},
        withdrawal::WithdrawalInfo,
    },
};

pub mod assignment;
pub mod deposit;
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
/// - `denomination` - The amount of bitcoin expected to be locked in the N/N multisig
/// - `deadline_duration` - The duration (in blocks) for assignment execution deadlines
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

    /// The amount of bitcoin expected to be locked in the N/N multisig.
    denomination: BitcoinAmount,

    /// The duration (in blocks) for assignment execution deadlines.
    deadline_duration: u64,
}

/// Configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1Config {
    /// Initial operator public keys for the bridge
    pub operators: Vec<OperatorPubkeys>,
    /// Expected deposit denomination for validation
    pub denomination: BitcoinAmount,
    /// Duration in blocks for assignment execution deadlines
    pub deadline_duration: u64,
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
    /// - `config` - Configuration containing operator public keys, denomination, and deadline duration
    ///
    /// # Returns
    ///
    /// A new [`BridgeV1State`] instance.
    pub fn new(config: &BridgeV1Config) -> Self {
        let operators = OperatorTable::from_operator_list(&config.operators);
        Self {
            operators,
            deposits: DepositsTable::new_empty(),
            assignments: AssignmentTable::new_empty(),
            denomination: config.denomination,
            deadline_duration: config.deadline_duration,
        }
    }

    /// Returns a reference to the operator table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`OperatorTable`].
    pub fn operators(&self) -> &crate::state::operator::OperatorTable {
        &self.operators
    }

    /// Returns a mutable reference to the operator table.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`OperatorTable`].
    pub fn operators_mut(&mut self) -> &mut crate::state::operator::OperatorTable {
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

    /// Returns the deadline duration for assignment execution.
    ///
    /// # Returns
    ///
    /// The duration (in blocks) for assignment execution deadlines.
    pub fn deadline_duration(&self) -> u64 {
        self.deadline_duration
    }

    /// Processes a deposit transaction by validating and adding it to the deposits table.
    ///
    /// This function takes already parsed deposit transaction information, validates it against the
    /// current state, and creates a new deposit entry in the deposits table if
    /// validation passes. Only operators that are part of the current N/N multisig set
    /// are included as notary operators for the deposit.
    ///
    /// # Parameters
    ///
    /// - `tx` - The deposit transaction
    /// - `info` - Parsed deposit information containing amount, destination, and outpoint
    ///
    /// # Returns
    ///
    /// - `Ok(u32)` - The deposit index assigned to the newly created deposit entry
    /// - `Err(DepositError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - The deposit amount is zero or negative
    /// - The internal key doesn't match the current aggregated operator key
    /// - The deposit index already exists in the deposits table
    pub fn process_deposit_tx(
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
        if info.amt.to_sat() != self.denomination.to_sat() {
            return Err(DepositError::InvalidDepositAmount {
                expected: self.denomination.to_sat(),
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

    /// Processes a withdrawal fulfillment transaction by validating it, removing the deposit, and
    /// removing the assignment.
    ///
    /// This function takes already parsed withdrawal transaction information, validates it against
    /// the current state using the assignment table, removes the corresponding deposit from the
    /// deposits table, and removes the assignment entry to mark the withdrawal as fulfilled.
    /// The withdrawal processing information is returned to the caller for storage in MohoState
    /// and later use by Bridge proof.
    ///
    /// # Parameters
    ///
    /// - `tx` - The withdrawal fulfillment transaction
    /// - `withdrawal_info` - Parsed withdrawal information containing operator, deposit details,
    ///   and withdrawal amounts
    ///
    /// # Returns
    ///
    /// - `Ok(WithdrawalProcessedInfo)` - The processed withdrawal information if transaction passes
    ///   validation
    /// - `Err(WithdrawalValidationError)` - If validation fails for any reason
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No assignment exists for the referenced deposit
    /// - The operator doesn't match the assigned operator
    /// - The withdrawal specifications don't match the assignment
    /// - The deposit referenced in the withdrawal doesn't exist
    pub fn process_withdrawal_fulfillment_tx(
        &mut self,
        tx: &Transaction,
        withdrawal_info: &WithdrawalInfo,
    ) -> Result<WithdrawalProcessedInfo, WithdrawalValidationError> {
        self.validate_withdrawal(withdrawal_info)?;

        // Remove the deposit from the deposits table since it's now fulfilled
        let removed_deposit = self
            .deposits
            .remove_deposit(withdrawal_info.deposit_idx)
            .expect("Deposit must exist after successful validation");

        // Remove the assignment from the table to mark withdrawal as fulfilled
        // Safe to unwrap since validate_withdrawal ensures the assignment exists
        let _removed_assignment = self
            .assignments
            .remove_assignment(withdrawal_info.deposit_idx)
            .expect("Assignment must exist after successful validation");

        Ok(WithdrawalProcessedInfo {
            withdrawal_txid: tx.compute_txid().into(),
            deposit_txid: removed_deposit.output().outpoint().txid.into(),
            deposit_idx: removed_deposit.idx(),
            operator_idx: withdrawal_info.operator_idx,
        })
    }

    /// Validates the parsed withdrawal it against assignment information.
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
    pub fn validate_withdrawal(
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

    /// Creates a withdrawal assignment by selecting an unassigned deposit and assigning it to an
    /// operator.
    ///
    /// This function handles incoming withdrawal commands by:
    /// 1. Finding a deposit that has not been assigned yet
    /// 2. Randomly selecting an operator from the current multisig set that is also a notary
    ///    operator for that deposit
    /// 3. Creating an assignment linking the deposit to the selected operator with a deadline
    ///    calculated from the current block height plus the configured deadline duration
    ///
    /// # Parameters
    ///
    /// - `withdrawal_cmd` - The withdrawal command specifying outputs and amounts
    /// - `l1_block_hash` - The L1 block hash used as seed for random operator selection
    /// - `current_block_height` - The current Bitcoin block height for deadline calculation
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal assignment was successfully created
    /// - `Err(WithdrawalCommandError)` - If no suitable deposit/operator combination could be found
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No unassigned deposits are available
    /// - No current multisig operators are notary operators for any unassigned deposit
    /// - The deposit for the unassigned index is not found
    pub fn create_withdrawal_assignment(
        &mut self,
        withdrawal_cmd: &WithdrawalCommand,
        l1_block_hash: &Buf32,
        current_block_height: u64,
    ) -> Result<(), WithdrawalCommandError> {
        // Find an unassigned deposit index
        let unassigned_deposit_idx = self.deposits().next_unassigned_idx();

        // Get the deposit to check its notary operators
        let deposit = self.deposits.get_deposit(unassigned_deposit_idx).ok_or(
            WithdrawalCommandError::DepositNotFound {
                deposit_idx: unassigned_deposit_idx,
            },
        )?;

        // Randomly select an operator from current multisig that is also in the deposit's notary
        // operators
        let selected_operator = self.select_operator_for_deposit(
            unassigned_deposit_idx,
            deposit.notary_operators(),
            l1_block_hash,
        )?;

        // Create assignment with deadline calculated from current block height + deadline duration
        let exec_deadline = current_block_height + self.deadline_duration();

        self.assignments.insert(
            unassigned_deposit_idx,
            withdrawal_cmd.clone(),
            selected_operator,
            exec_deadline,
        );

        // Increment the next unassigned index since we just created an assignment for this deposit
        self.deposits.increment_next_unassigned_idx();

        Ok(())
    }

    /// Randomly selects an operator from the current multisig set that is also a notary operator
    /// for the deposit.
    ///
    /// Uses ChaChaRng with the L1 block hash as seed to ensure deterministic but unpredictable
    /// operator selection across different nodes.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index for error reporting
    /// - `notary_operators` - List of notary operator indices for the deposit
    /// - `l1_block_hash` - The L1 block hash used as seed for random selection
    ///
    /// # Returns
    ///
    /// - `Ok(OperatorIdx)` - Index of a randomly selected suitable operator
    /// - `Err(WithdrawalCommandError)` - If no current multisig operator is found in the notary
    ///   operators
    fn select_operator_for_deposit(
        &self,
        deposit_idx: u32,
        notary_operators: &[u32],
        l1_block_hash: &Buf32,
    ) -> Result<u32, WithdrawalCommandError> {
        // Collect current multisig operators into a small Vec for efficient contains() check
        let current_multisig_operators: Vec<u32> =
            self.operators.current_multisig_indices().collect();

        // Filter notary operators to only those in current multisig
        let eligible_operators: Vec<u32> = notary_operators
            .iter()
            .filter(|&&op_idx| current_multisig_operators.contains(&op_idx))
            .copied()
            .collect();

        if eligible_operators.is_empty() {
            return Err(WithdrawalCommandError::NoEligibleOperators { deposit_idx });
        }

        // Use ChaChaRng with L1 block hash as seed for deterministic random selection
        let seed_bytes: [u8; 32] = (*l1_block_hash).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        // Select a random index
        let random_index = (rng.next_u32() as usize) % eligible_operators.len();

        Ok(eligible_operators[random_index])
    }
}
