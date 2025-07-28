//! Operator Assignment Management
//!
//! This module contains types and tables for managing operator assignments to deposits.
//! Assignments link specific deposits to operators who are responsible for processing
//! withdrawal requests within specified deadlines.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use rand_chacha::{
    ChaChaRng,
    rand_core::{RngCore, SeedableRng},
};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::{BitcoinBlockHeight, OperatorIdx},
    buf::Buf32,
    l1::L1BlockId,
};

use super::withdrawal::WithdrawalCommand;
use crate::{errors::WithdrawalCommandError, state::deposit::DepositEntry};

/// Assignment entry linking a deposit to an operator for withdrawal processing.
///
/// Each assignment represents a task assigned to a specific operator to process
/// a withdrawal from a particular deposit. The assignment includes:
///
/// - **`deposit_idx`** - Reference to the deposit being processed
/// - **`withdrawal_cmd`** - Specification of outputs and amounts for withdrawal
/// - **`current_assignee`** - Operator currently responsible for executing the withdrawal
/// - **`previous_assignees`** - List of operators who were previously assigned but failed to
///   execute
/// - **`exec_deadline`** - Bitcoin block height deadline for execution
///
/// # Execution Deadline
///
/// The `exec_deadline` represents the latest Bitcoin block height at which
/// the withdrawal can be executed. If a checkpoint is processed after this
/// height and the withdrawal hasn't been completed, the assignment becomes invalid.
#[derive(
    Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Arbitrary,
)]
pub struct AssignmentEntry {
    deposit_entry: DepositEntry,

    /// Withdrawal command specifying outputs and amounts.
    withdrawal_cmd: WithdrawalCommand,

    /// Index of the operator currently assigned to execute this withdrawal.
    ///
    /// This operator fronts the funds for the withdrawal and will be
    /// reimbursed by the bridge notaries upon successful execution.
    current_assignee: OperatorIdx,

    /// List of operators who were previously assigned to this withdrawal.
    ///
    /// When a withdrawal is reassigned, the current assignee is moved to this
    /// list before a new operator is selected. This prevents reassigning to
    /// operators who have already failed to execute the withdrawal.
    previous_assignees: Vec<OperatorIdx>,

    /// Bitcoin block height deadline for withdrawal execution.
    ///
    /// The withdrawal must be executed before this block height.
    /// If a checkpoint is processed at or after this height without
    /// the withdrawal being completed, the assignment becomes invalid.
    exec_deadline: BitcoinBlockHeight,
}

fn select_eligible_operators(
    notary_operators: &[OperatorIdx],
    previous_assignees: &[OperatorIdx],
    current_active_operators: &[OperatorIdx],
) -> Vec<OperatorIdx> {
    // Filter out previous assignees to get eligible operators
    notary_operators
        .iter()
        .filter(|&&op| !previous_assignees.contains(&op))
        .filter(|&&op| current_active_operators.contains(&op))
        .cloned()
        .collect()
}

impl AssignmentEntry {
    /// Creates a new assignment entry by randomly selecting an eligible operator.
    ///
    /// Performs deterministic random selection of an operator from the deposit's notary set,
    /// filtering by currently active operators. Uses the provided L1 block ID as a seed
    /// for reproducible operator assignment across nodes.
    ///
    /// # Parameters
    ///
    /// - `deposit_entry` - The deposit entry to be processed
    /// - `withdrawal_cmd` - Withdrawal command with output specifications
    /// - `exec_deadline` - Bitcoin block height deadline for execution
    /// - `current_active_operators` - Slice of currently active operator indices
    /// - `seed` - L1 block ID used as seed for deterministic random selection
    ///
    /// # Returns
    ///
    /// - `Ok(AssignmentEntry)` - A new assignment entry with randomly selected operator
    /// - `Err(WithdrawalCommandError::NoEligibleOperators)` - If no eligible operators are
    ///   available
    pub fn create_with_random_assignment(
        deposit_entry: DepositEntry,
        withdrawal_cmd: WithdrawalCommand,
        exec_deadline: BitcoinBlockHeight,
        current_active_operators: &[OperatorIdx],
        seed: L1BlockId,
    ) -> Result<Self, WithdrawalCommandError> {
        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        let eligible_operators = select_eligible_operators(
            deposit_entry.notary_operators(),
            &[], // No previous assignees at creation
            current_active_operators,
        );

        if eligible_operators.is_empty() {
            return Err(WithdrawalCommandError::NoEligibleOperators {
                deposit_idx: deposit_entry.idx(),
            });
        }

        // Select a random index
        let random_index = (rng.next_u32() as usize) % eligible_operators.len();
        let current_assignee = eligible_operators[random_index];

        Ok(Self {
            deposit_entry,
            withdrawal_cmd,
            current_assignee,
            previous_assignees: Vec::new(),
            exec_deadline,
        })
    }

    /// Returns the deposit index associated with this assignment.
    ///
    /// # Returns
    ///
    /// The deposit index as [`u32`].
    pub fn deposit_idx(&self) -> u32 {
        self.deposit_entry.idx()
    }

    pub fn deposit_txid(&self) -> Buf32 {
        self.deposit_entry.output().outpoint().txid.into()
    }

    /// Returns a reference to the withdrawal command.
    ///
    /// # Returns
    ///
    /// Reference to the [`WithdrawalCommand`] containing output specifications.
    pub fn withdrawal_command(&self) -> &WithdrawalCommand {
        &self.withdrawal_cmd
    }

    /// Returns the index of the currently assigned operator.
    ///
    /// # Returns
    ///
    /// The [`OperatorIdx`] of the operator currently responsible for this assignment.
    pub fn current_assignee(&self) -> OperatorIdx {
        self.current_assignee
    }

    /// Returns a reference to the list of previous assignees.
    ///
    /// # Returns
    ///
    /// Reference to the [`Vec<OperatorIdx>`] of operators previously assigned to this withdrawal.
    pub fn previous_assignees(&self) -> &[OperatorIdx] {
        &self.previous_assignees
    }

    /// Returns the execution deadline for this assignment.
    ///
    /// # Returns
    ///
    /// The [`BitcoinBlockHeight`] deadline for withdrawal execution.
    pub fn exec_deadline(&self) -> BitcoinBlockHeight {
        self.exec_deadline
    }

    /// Reassigns the withdrawal to a new randomly selected operator.
    ///
    /// Moves the current assignee to the previous assignees list and randomly selects
    /// a new operator from eligible candidates. If no eligible operators remain (all
    /// have been tried), clears the previous assignees list and selects from all
    /// active notary operators.
    ///
    /// # Parameters
    ///
    /// - `seed` - L1 block ID used as seed for deterministic random selection
    /// - `current_active_operators` - Slice of currently active operator indices
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If the reassignment succeeded
    /// - `Err(WithdrawalCommandError)` - If no eligible operators are available
    pub fn reassign(
        &mut self,
        seed: L1BlockId,
        current_active_operators: &[OperatorIdx],
    ) -> Result<(), WithdrawalCommandError> {
        self.previous_assignees.push(self.current_assignee);

        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        let mut eligible_operators = select_eligible_operators(
            self.deposit_entry.notary_operators(),
            &self.previous_assignees,
            current_active_operators,
        );

        if eligible_operators.is_empty() {
            // If no eligible operators left, clear previous assignees
            self.previous_assignees.clear();
            eligible_operators = select_eligible_operators(
                self.deposit_entry.notary_operators(),
                &self.previous_assignees,
                current_active_operators,
            );
        }

        // If still no eligible operators, return error
        if eligible_operators.is_empty() {
            return Err(WithdrawalCommandError::NoEligibleOperators {
                deposit_idx: self.deposit_entry.idx(),
            });
        }

        // Select a random index
        let random_index = (rng.next_u32() as usize) % eligible_operators.len();
        let new_assignee = eligible_operators[random_index];

        self.current_assignee = new_assignee;
        Ok(())
    }
}

/// Table for managing operator assignments with efficient lookup operations.
///
/// This table maintains all assignments linking deposits to operators, providing
/// efficient insertion, lookup, and filtering operations. The table maintains
/// sorted order for binary search efficiency.
///
/// # Ordering Invariant
///
/// The assignments vector **MUST** remain sorted by deposit index at all times.
/// This invariant enables O(log n) lookup operations via binary search.
///
/// # Assignment Management
///
/// The table supports various operations including:
/// - Creating new assignments with optimized insertion
/// - Looking up assignments by deposit index
/// - Filtering assignments by operator or expiration status
/// - Removing completed or expired assignments
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct AssignmentTable {
    /// Vector of assignment entries, sorted by deposit index.
    ///
    /// **Invariant**: MUST be sorted by `AssignmentEntry::deposit_idx` field.
    assignments: Vec<AssignmentEntry>,
}

impl AssignmentTable {
    /// Creates a new empty assignment table.
    ///
    /// Initializes the table with no assignments, ready for operator
    /// assignment management.
    ///
    /// # Returns
    ///
    /// A new empty [`AssignmentTable`].
    pub fn new_empty() -> Self {
        Self {
            assignments: Vec::new(),
        }
    }

    /// Returns the number of assignments in the table.
    ///
    /// # Returns
    ///
    /// The total count of assignments as [`u32`].
    pub fn len(&self) -> u32 {
        self.assignments.len() as u32
    }

    /// Returns whether the assignment table is empty.
    ///
    /// # Returns
    ///
    /// `true` if no assignments exist, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// Returns a slice of all assignment entries.
    ///
    /// The entries are guaranteed to be sorted by deposit index.
    ///
    /// # Returns
    ///
    /// Slice reference to all [`AssignmentEntry`] instances in the table.
    pub fn assignments(&self) -> &[AssignmentEntry] {
        &self.assignments
    }

    /// Retrieves an assignment entry by its deposit index.
    ///
    /// Uses binary search for O(log n) lookup performance.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&AssignmentEntry)` if the assignment exists
    /// - `None` if no assignment for the given deposit index is found
    pub fn get_assignment(&self, deposit_idx: u32) -> Option<&AssignmentEntry> {
        self.assignments
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx())
            .ok()
            .map(|i| &self.assignments[i])
    }

    /// Creates a new assignment entry with optimized insertion.
    ///
    /// This method is optimized for sequential insertion patterns. If the deposit
    /// index is larger than the last entry, it performs a fast append operation.
    /// Otherwise, it uses binary search to find the correct insertion position.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - Deposit index for the assignment
    /// - `withdrawal_cmd` - Withdrawal command with output specifications
    /// - `assignee` - Operator index assigned to execute the withdrawal
    /// - `exec_deadline` - Bitcoin block height deadline for execution
    ///
    /// # Panics
    ///
    /// Panics if an assignment with the given deposit index already exists.
    pub fn insert(&mut self, entry: AssignmentEntry) {
        let idx = entry.deposit_idx();

        // Fast path: if this is larger than the last entry, just push
        if let Some(last) = self.assignments.last() {
            if idx > last.deposit_idx() {
                self.assignments.push(entry);
                return;
            }
        } else {
            // Empty table, just push
            self.assignments.push(entry);
            return;
        }

        // Perform binary search to find the insertion point
        match self
            .assignments
            .binary_search_by_key(&idx, |entry| entry.deposit_idx())
        {
            Ok(_) => panic!("Assignment with deposit index {} already exists", idx),
            Err(pos) => {
                // Insert the deposit entry at the found position
                self.assignments.insert(pos, entry);
            }
        }
    }

    /// Removes an assignment by its deposit index.
    ///
    /// Uses binary search to locate and remove the assignment efficiently.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index of the assignment to remove
    ///
    /// # Returns
    ///
    /// - `Some(AssignmentEntry)` if the assignment was found and removed
    /// - `None` if no assignment with the given deposit index exists
    pub fn remove_assignment(&mut self, deposit_idx: u32) -> Option<AssignmentEntry> {
        let pos = self
            .assignments
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx())
            .ok()?;

        // Remove the assignment and return it
        Some(self.assignments.remove(pos))
    }

    /// Reassigns all expired assignments to new randomly selected operators.
    ///
    /// Iterates through all assignments and reassigns those whose execution deadlines
    /// have passed (current height >= exec_deadline). Each expired assignment is
    /// reassigned using the provided seed for deterministic random operator selection.
    ///
    /// This method handles bulk reassignment of expired assignments, ensuring that
    /// withdrawals don't get stuck due to unresponsive operators. If any individual
    /// reassignment fails (e.g., no eligible operators), the entire operation fails
    /// and returns an error.
    ///
    /// # Parameters
    ///
    /// - `current_height` - The current Bitcoin block height for expiration comparison
    /// - `current_active_operators` - Slice of currently active operator indices
    /// - `seed` - L1 block ID used as seed for deterministic random selection
    ///
    /// # Returns
    ///
    /// - `Ok(())` - If all expired assignments were successfully reassigned
    /// - `Err(WithdrawalCommandError)` - If any reassignment failed due to lack of eligible
    ///   operators
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let current_height = BitcoinBlockHeight::from(1000);
    /// let active_operators = vec![1, 2, 3];
    /// let seed = L1BlockId::from([0u8; 32]);
    ///
    /// table.reassign_expired_assignments(current_height, &active_operators, seed)?;
    /// ```
    pub fn reassign_expired_assignments(
        &mut self,
        current_height: BitcoinBlockHeight,
        current_active_operators: &[OperatorIdx],
        seed: L1BlockId,
    ) -> Result<(), WithdrawalCommandError> {
        for assignment in self
            .assignments
            .iter_mut()
            .filter(|e| e.exec_deadline <= current_height)
        {
            assignment.reassign(seed, current_active_operators)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{bridge::BitcoinBlockHeight, l1::L1BlockId};
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_create_with_random_assignment_success() {
        let mut arb = ArbitraryGenerator::new();
        let deposit_entry: DepositEntry = arb.generate();
        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed: L1BlockId = arb.generate();

        // Use the deposit's notary operators as active operators
        let current_active_operators: Vec<OperatorIdx> = deposit_entry.notary_operators().to_vec();

        let result = AssignmentEntry::create_with_random_assignment(
            deposit_entry.clone(),
            withdrawal_cmd.clone(),
            exec_deadline,
            &current_active_operators,
            seed,
        );

        assert!(result.is_ok());
        let assignment = result.unwrap();

        // Verify assignment properties
        assert_eq!(assignment.deposit_idx(), deposit_entry.idx());
        assert_eq!(assignment.withdrawal_command(), &withdrawal_cmd);
        assert_eq!(assignment.exec_deadline(), exec_deadline);
        assert!(current_active_operators.contains(&assignment.current_assignee()));
        assert_eq!(assignment.previous_assignees().len(), 0);
    }

    #[test]
    fn test_create_with_random_assignment_no_eligible_operators() {
        let mut arb = ArbitraryGenerator::new();
        let deposit_entry: DepositEntry = arb.generate();
        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed: L1BlockId = arb.generate();

        // Empty active operators list
        let current_active_operators: Vec<OperatorIdx> = vec![];

        let result = AssignmentEntry::create_with_random_assignment(
            deposit_entry.clone(),
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WithdrawalCommandError::NoEligibleOperators { .. }
        ));
    }

    #[test]
    fn test_reassign_success() {
        let mut arb = ArbitraryGenerator::new();
        let deposit_entry: DepositEntry = arb.generate();
        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed1: L1BlockId = arb.generate();
        let seed2: L1BlockId = arb.generate();

        // Use the deposit's notary operators as active operators
        let current_active_operators: Vec<OperatorIdx> = deposit_entry.notary_operators().to_vec();

        // Ensure we have at least 2 operators for reassignment
        if current_active_operators.len() < 2 {
            return; // Skip test if not enough operators
        }

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        let original_assignee = assignment.current_assignee();
        assert_eq!(assignment.previous_assignees().len(), 0);

        // Reassign to a new operator
        let result = assignment.reassign(seed2, &current_active_operators);
        assert!(result.is_ok());

        // Verify reassignment
        assert_eq!(assignment.previous_assignees().len(), 1);
        assert_eq!(assignment.previous_assignees()[0], original_assignee);
        assert_ne!(assignment.current_assignee(), original_assignee);
        assert!(current_active_operators.contains(&assignment.current_assignee()));
    }

    #[test]
    fn test_reassign_all_operators_exhausted() {
        let mut arb = ArbitraryGenerator::new();
        let mut deposit_entry: DepositEntry = arb.generate();

        // Force single operator for this test
        let single_operator: OperatorIdx = 1;
        let operators = vec![single_operator];
        deposit_entry = DepositEntry::new(
            deposit_entry.idx(),
            deposit_entry.output().clone(),
            operators.clone(),
            deposit_entry.amt(),
        )
        .unwrap();

        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed1: L1BlockId = arb.generate();
        let seed2: L1BlockId = arb.generate();

        let current_active_operators = vec![single_operator];

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        // First reassignment should work (clears previous assignees and reassigns to same operator)
        let result = assignment.reassign(seed2, &current_active_operators);
        assert!(result.is_ok());

        // Should have cleared previous assignees and reassigned to the same operator
        assert_eq!(assignment.previous_assignees().len(), 0);
        assert_eq!(assignment.current_assignee(), single_operator);
    }

    #[test]
    fn test_assignment_table_basic_operations() {
        let mut table = AssignmentTable::new_empty();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        let mut arb = ArbitraryGenerator::new();
        let deposit_entry: DepositEntry = arb.generate();
        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed: L1BlockId = arb.generate();
        let current_active_operators: Vec<OperatorIdx> = deposit_entry.notary_operators().to_vec();

        let assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry.clone(),
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed,
        )
        .unwrap();

        let deposit_idx = assignment.deposit_idx();

        // Insert assignment
        table.insert(assignment.clone());
        assert!(!table.is_empty());
        assert_eq!(table.len(), 1);

        // Get assignment
        let retrieved = table.get_assignment(deposit_idx);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().deposit_idx(), deposit_idx);

        // Remove assignment
        let removed = table.remove_assignment(deposit_idx);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().deposit_idx(), deposit_idx);
        assert!(table.is_empty());
    }

    #[test]
    fn test_reassign_expired_assignments() {
        let mut table = AssignmentTable::new_empty();
        let mut arb = ArbitraryGenerator::new();

        // Create test data
        let current_height: BitcoinBlockHeight = 150;
        let seed: L1BlockId = arb.generate();

        // Create expired assignment (deadline < current_height)
        let deposit_entry1: DepositEntry = arb.generate();
        let withdrawal_cmd1: WithdrawalCommand = arb.generate();
        let expired_deadline: BitcoinBlockHeight = 100; // Less than current_height
        let current_active_operators1: Vec<OperatorIdx> =
            deposit_entry1.notary_operators().to_vec();

        let expired_assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry1.clone(),
            withdrawal_cmd1,
            expired_deadline,
            &current_active_operators1,
            seed,
        )
        .unwrap();

        let expired_deposit_idx = expired_assignment.deposit_idx();
        let original_assignee = expired_assignment.current_assignee();
        table.insert(expired_assignment);

        // Create non-expired assignment (deadline > current_height)
        let deposit_entry2: DepositEntry = arb.generate();
        let withdrawal_cmd2: WithdrawalCommand = arb.generate();
        let future_deadline: BitcoinBlockHeight = 200; // Greater than current_height
        let current_active_operators2: Vec<OperatorIdx> =
            deposit_entry2.notary_operators().to_vec();

        let future_assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry2.clone(),
            withdrawal_cmd2,
            future_deadline,
            &current_active_operators2,
            seed,
        )
        .unwrap();

        let future_deposit_idx = future_assignment.deposit_idx();
        let future_original_assignee = future_assignment.current_assignee();
        table.insert(future_assignment);

        // Reassign expired assignments
        let result =
            table.reassign_expired_assignments(current_height, &current_active_operators1, seed);

        assert!(result.is_ok(), "Reassignment should succeed");

        // Check that expired assignment was reassigned
        let expired_assignment_after = table.get_assignment(expired_deposit_idx).unwrap();
        assert_eq!(expired_assignment_after.previous_assignees().len(), 1);
        assert_eq!(
            expired_assignment_after.previous_assignees()[0],
            original_assignee
        );

        // Check that non-expired assignment was not reassigned
        let future_assignment_after = table.get_assignment(future_deposit_idx).unwrap();
        assert_eq!(future_assignment_after.previous_assignees().len(), 0);
        assert_eq!(
            future_assignment_after.current_assignee(),
            future_original_assignee
        );
    }
}
