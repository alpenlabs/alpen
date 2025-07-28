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
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, L1BlockId},
    operator::OperatorPubkeys,
};

use crate::{
    errors::{DepositError, WithdrawalCommandError, WithdrawalValidationError},
    state::{
        assignment::AssignmentTable,
        deposit::DepositsTable,
        withdrawal::{WithdrawalCommand, WithdrawalProcessedInfo},
    },
    txs::{
        deposit::{
            parse::DepositInfo,
            validation::{validate_deposit_output_lock, validate_drt_spending_signature},
        },
        withdrawal_fulfillment::WithdrawalInfo,
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

    /// Returns a reference to the deposits table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`DepositsTable`].
    pub fn deposits(&self) -> &DepositsTable {
        &self.deposits
    }

    /// Returns a reference to the assignments table.
    ///
    /// # Returns
    ///
    /// Immutable reference to the [`AssignmentTable`].
    pub fn assignments(&self) -> &AssignmentTable {
        &self.assignments
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
        let expected_operator = assignment.current_assignee();
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

        let expected_txid = deposit.output().outpoint().txid.into();
        let actual_txid = withdrawal_info.deposit_txid.clone();

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
    /// - `l1_block_id` - The L1 block ID used as seed for random operator selection
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
        l1_block_id: &L1BlockId,
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
        let selected_operator = self.select_operator_for_deposit_excluding(
            unassigned_deposit_idx,
            deposit.notary_operators(),
            &[],
            l1_block_id,
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

    /// Reassigns a withdrawal to a new operator by moving the current assignee to previous
    /// assignees and selecting a new eligible operator.
    ///
    /// This function handles withdrawal reassignment by:
    /// 1. Finding the existing assignment for the deposit
    /// 2. Getting the deposit to check notary operators
    /// 3. Selecting a new operator that hasn't been assigned before
    /// 4. Reassigning the withdrawal to the new operator
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index to reassign
    /// - `l1_block_id` - The L1 block ID used as seed for random operator selection
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the withdrawal was successfully reassigned
    /// - `Err(WithdrawalCommandError)` - If reassignment failed
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No assignment exists for the deposit
    /// - The deposit is not found
    /// - No eligible operators are available for reassignment
    pub fn reassign_withdrawal(
        &mut self,
        deposit_idx: u32,
        l1_block_id: &L1BlockId,
    ) -> Result<(), WithdrawalCommandError> {
        // Get the existing assignment
        let assignment = self
            .assignments
            .get_assignment(deposit_idx)
            .ok_or(WithdrawalCommandError::DepositNotFound { deposit_idx })?;

        // Get the deposit to check its notary operators
        let deposit = self
            .deposits
            .get_deposit(deposit_idx)
            .ok_or(WithdrawalCommandError::DepositNotFound { deposit_idx })?;

        // Collect all previous assignees including the current one
        let mut excluded_operators = assignment.previous_assignees().to_vec();
        excluded_operators.push(assignment.current_assignee());

        // Select a new operator excluding previous assignees
        let selected_operator = self.select_operator_for_deposit_excluding(
            deposit_idx,
            deposit.notary_operators(),
            &excluded_operators,
            l1_block_id,
        )?;

        // Reassign the withdrawal
        let assignment = self
            .assignments
            .get_assignment_mut(deposit_idx)
            .expect("Assignment exists since we found it above");
        assignment.reassign(selected_operator);

        Ok(())
    }

    /// Randomly selects an operator from the current multisig set that is also a notary operator
    /// for the deposit, excluding operators that have been previously assigned.
    ///
    /// Uses ChaChaRng with the L1 block ID as seed to ensure deterministic but unpredictable
    /// operator selection across different nodes.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index for error reporting
    /// - `notary_operators` - List of notary operator indices for the deposit
    /// - `excluded_operators` - List of operator indices to exclude from selection
    /// - `l1_block_id` - The L1 block ID used as seed for random selection
    ///
    /// # Returns
    ///
    /// - `Ok(OperatorIdx)` - Index of a randomly selected suitable operator
    /// - `Err(WithdrawalCommandError)` - If no eligible operator is found
    fn select_operator_for_deposit_excluding(
        &self,
        deposit_idx: u32,
        notary_operators: &[u32],
        excluded_operators: &[u32],
        l1_block_id: &L1BlockId,
    ) -> Result<u32, WithdrawalCommandError> {
        // Collect current multisig operators into a small Vec for efficient contains() check
        let current_multisig_operators: Vec<u32> =
            self.operators.current_multisig_indices().collect();

        // Filter notary operators to only those in current multisig and not excluded
        let eligible_operators: Vec<u32> = notary_operators
            .iter()
            .filter(|&&op_idx| {
                current_multisig_operators.contains(&op_idx)
                    && !excluded_operators.contains(&op_idx)
            })
            .copied()
            .collect();

        if eligible_operators.is_empty() {
            return Err(WithdrawalCommandError::NoEligibleOperators { deposit_idx });
        }

        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(*l1_block_id).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        // Select a random index
        let random_index = (rng.next_u32() as usize) % eligible_operators.len();

        Ok(eligible_operators[random_index])
    }

    /// Selects an operator for a deposit with fallback to clearing previous assignees.
    ///
    /// This function attempts to select an operator excluding previously assigned ones.
    /// If no eligible operators are found, it clears all previous assignees from the
    /// assignment and tries again with all notary operators available.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index for the assignment
    /// - `notary_operators` - List of notary operator indices for the deposit
    /// - `excluded_operators` - List of operator indices to exclude from selection
    /// - `l1_block_id` - The L1 block ID used as seed for random selection
    ///
    /// # Returns
    ///
    /// - `u32` - Index of a randomly selected suitable operator
    ///
    /// # Behavior
    ///
    /// 1. First attempts to select from operators not in `excluded_operators`
    /// 2. If no eligible operators found, clears the assignment's previous assignees
    /// 3. Retries selection with all notary operators available
    /// 4. This ensures that withdrawals can always be reassigned when operators are available
    fn select_operator_for_deposit_with_fallback(
        &mut self,
        deposit_idx: u32,
        notary_operators: &[u32],
        excluded_operators: &[u32],
        l1_block_id: &L1BlockId,
    ) -> u32 {
        // First attempt: try to select excluding previous assignees
        match self.select_operator_for_deposit_excluding(
            deposit_idx,
            notary_operators,
            excluded_operators,
            l1_block_id,
        ) {
            Ok(operator_idx) => operator_idx,
            Err(WithdrawalCommandError::NoEligibleOperators { .. }) => {
                // No eligible operators found - clear previous assignees and try again
                if let Some(assignment) = self.assignments.get_assignment_mut(deposit_idx) {
                    assignment.previous_assignees_mut().clear();
                }

                // Retry with no exclusions (all notary operators are now eligible)
                self.select_operator_for_deposit_excluding(
                    deposit_idx,
                    notary_operators,
                    &[], // No exclusions this time
                    l1_block_id,
                )
                .expect("Should always find an operator after clearing previous assignees")
            }
            Err(other_error) => {
                // For other errors (like deposit not found), we can't recover
                panic!("Unexpected error in operator selection: {other_error}");
            }
        }
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
        current_block: &strata_primitives::l1::L1BlockCommitment,
    ) -> Result<Vec<u32>, WithdrawalCommandError> {
        let current_block_height = current_block.height();
        let l1_block_id = current_block.blkid();

        // Collect expired assignment deposit indices first to avoid borrowing issues
        let expired_deposit_indices: Vec<u32> = self
            .assignments
            .get_expired_assignments(current_block_height)
            .map(|assignment| assignment.deposit_idx())
            .collect();

        let mut reassigned_deposits = Vec::new();

        for deposit_idx in expired_deposit_indices {
            self.reassign_withdrawal(deposit_idx, l1_block_id)?;
            reassigned_deposits.push(deposit_idx);
        }

        Ok(reassigned_deposits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::SecretKey;
    use rand::rngs::OsRng;
    use strata_primitives::{buf::Buf32, l1::BitcoinAmount, operator::OperatorPubkeys};
    use crate::txs::deposit::{create::create_test_deposit_tx, parse::DepositInfo};

    /// Helper function to create a test BridgeV1State with operators
    fn create_test_bridge_state(num_operators: usize, denomination: u64) -> (BridgeV1State, Vec<SecretKey>) {
        let mut operator_privkeys = Vec::new();
        let mut operator_pubkeys = Vec::new();

        // Generate operator keys
        for _ in 0..num_operators {
            let privkey = SecretKey::new(&mut OsRng);
            let pubkey = bitcoin::secp256k1::PublicKey::from_secret_key(&bitcoin::secp256k1::Secp256k1::new(), &privkey);
            let xonly_pubkey = pubkey.x_only_public_key().0;
            
            operator_privkeys.push(privkey);
            operator_pubkeys.push(OperatorPubkeys::new(
                Buf32::from(xonly_pubkey.serialize()),  // signing_pk
                Buf32::from(xonly_pubkey.serialize()),  // wallet_pk (same for simplicity)
            ));
        }

        let config = BridgeV1Config {
            operators: operator_pubkeys,
            denomination: BitcoinAmount::from_sat(denomination),
            deadline_duration: 144, // 1 day in blocks
        };

        let state = BridgeV1State::new(&config);
        (state, operator_privkeys)
    }

    /// Helper function to create test deposit info
    fn create_test_deposit_info(deposit_idx: u32, amount: u64) -> DepositInfo {
        DepositInfo {
            deposit_idx,
            amt: BitcoinAmount::from_sat(amount),
            address: b"test-address".to_vec(),
            outpoint: bitcoin::OutPoint::null().into(),
            drt_tapnode_hash: Buf32::from([0u8; 32]),
        }
    }

    /// Helper function to create unique deposit info with different outpoints
    fn create_unique_deposit_info(deposit_idx: u32, amount: u64, tx_index: u32) -> DepositInfo {
        let mut outpoint = bitcoin::OutPoint::null();
        outpoint.vout = tx_index; // Make each outpoint unique
        DepositInfo {
            deposit_idx,
            amt: BitcoinAmount::from_sat(amount),
            address: format!("test-address-{}", tx_index).into_bytes(),
            outpoint: outpoint.into(),
            drt_tapnode_hash: Buf32::from([tx_index as u8; 32]),
        }
    }

    #[test]
    fn test_process_deposit_tx_success() {
        let denomination = 100_000; // 0.001 BTC
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        let deposit_info = create_test_deposit_info(1, denomination);
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        
        assert!(result.is_ok());
        let deposit_idx = result.unwrap();
        assert_eq!(deposit_idx, 0); // First deposit gets index 0
        
        // Verify deposit was added to the table
        let deposit = state.deposits().get_deposit(deposit_idx);
        assert!(deposit.is_some());
        let deposit = deposit.unwrap();
        assert_eq!(deposit.idx(), deposit_idx);
    }

    #[test]
    fn test_process_deposit_tx_invalid_amount() {
        let denomination = 100_000;
        let wrong_amount = 50_000; // Different amount
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        let deposit_info = create_test_deposit_info(1, wrong_amount);
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        
        assert!(result.is_err());
        match result.unwrap_err() {
            DepositError::InvalidDepositAmount { expected, actual } => {
                assert_eq!(expected, denomination);
                assert_eq!(actual, wrong_amount);
            }
            _ => panic!("Expected InvalidDepositAmount error"),
        }
    }

    #[test]
    fn test_process_deposit_tx_duplicate_index() {
        let denomination = 100_000;
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        // First create a deposit that will get index 0
        let deposit_info1 = create_unique_deposit_info(1, denomination, 1);
        let tx1 = create_test_deposit_tx(&deposit_info1, &operator_privkeys);
        let result1 = state.process_deposit_tx(&tx1, &deposit_info1);
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap(), 0); // Gets auto-incremented index 0
        
        // Now try to create another deposit with deposit_idx=0 (which already exists)
        let deposit_info2 = create_unique_deposit_info(0, denomination, 2); // deposit_idx=0 conflicts
        let tx2 = create_test_deposit_tx(&deposit_info2, &operator_privkeys);
        let result2 = state.process_deposit_tx(&tx2, &deposit_info2);
        
        assert!(result2.is_err());
        match result2.unwrap_err() {
            DepositError::DepositIdxAlreadyExists(idx) => {
                assert_eq!(idx, 0);
            }
            _ => panic!("Expected DepositIdxAlreadyExists error"),
        }
    }

    #[test]
    fn test_process_deposit_tx_multiple_deposits() {
        let denomination = 100_000;
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        // Process multiple deposits with different indices
        for i in 1..=5 {
            let deposit_info = create_unique_deposit_info(i, denomination, i);
            let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
            
            let result = state.process_deposit_tx(&tx, &deposit_info);
            assert!(result.is_ok());
            // Auto-incremented index starts from 0, so deposit i gets index i-1
            let expected_index = i - 1;
            assert_eq!(result.unwrap(), expected_index);
            
            // Verify each deposit exists
            let deposit = state.deposits().get_deposit(expected_index);
            assert!(deposit.is_some());
        }
    }

    #[test]
    fn test_process_deposit_tx_notary_operators_assigned() {
        let denomination = 100_000;
        let (mut state, operator_privkeys) = create_test_bridge_state(5, denomination);
        
        let deposit_info = create_test_deposit_info(1, denomination);
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        assert!(result.is_ok());
        
        let deposit = state.deposits().get_deposit(0).unwrap(); // First deposit gets index 0
        let notary_operators = deposit.notary_operators();
        
        // All current multisig operators should be notary operators
        let current_multisig: Vec<u32> = state.operators().current_multisig_indices().collect();
        assert_eq!(notary_operators.len(), current_multisig.len());
        
        for &op_idx in notary_operators {
            assert!(current_multisig.contains(&op_idx));
        }
    }

    #[test]
    fn test_process_deposit_tx_zero_amount() {
        let denomination = 100_000;
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        let deposit_info = create_test_deposit_info(1, 0); // Zero amount
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        
        assert!(result.is_err());
        match result.unwrap_err() {
            DepositError::InvalidDepositAmount { expected, actual } => {
                assert_eq!(expected, denomination);
                assert_eq!(actual, 0);
            }
            _ => panic!("Expected InvalidDepositAmount error"),
        }
    }

    #[test] 
    fn test_process_deposit_tx_single_operator() {
        let denomination = 100_000;
        let (mut state, operator_privkeys) = create_test_bridge_state(1, denomination);
        
        let deposit_info = create_test_deposit_info(1, denomination);
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        assert!(result.is_ok());
        
        let deposit = state.deposits().get_deposit(0).unwrap(); // First deposit gets index 0
        assert_eq!(deposit.notary_operators().len(), 1);
        assert_eq!(deposit.notary_operators()[0], 0); // First operator index
    }

    #[test]
    fn test_process_deposit_tx_large_denomination() {
        let denomination = 2_100_000_000_000; // 21,000 BTC in satoshis
        let (mut state, operator_privkeys) = create_test_bridge_state(3, denomination);
        
        let deposit_info = create_test_deposit_info(1, denomination);
        let tx = create_test_deposit_tx(&deposit_info, &operator_privkeys);
        
        let result = state.process_deposit_tx(&tx, &deposit_info);
        assert!(result.is_ok());
        
        let deposit = state.deposits().get_deposit(0).unwrap(); // First deposit gets index 0
        assert_eq!(deposit.amt().to_sat(), denomination);
    }

    #[test]
    fn test_validate_deposit_invalid_signature() {
        let denomination = 100_000;
        let (state, _) = create_test_bridge_state(3, denomination);
        
        // Create deposit info with different operators (invalid signature)
        let wrong_operators: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let deposit_info = create_test_deposit_info(1, denomination);
        let tx = create_test_deposit_tx(&deposit_info, &wrong_operators);
        
        let result = state.validate_deposit(&tx, &deposit_info);
        
        assert!(result.is_err());
        // Should fail with signature validation error
        match result.unwrap_err() {
            DepositError::InvalidSignature { .. } => {
                // Expected
            }
            other => panic!("Expected InvalidSignature error, got: {:?}", other),
        }
    }
}
