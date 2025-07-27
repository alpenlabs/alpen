//! Operator Assignment Management
//!
//! This module contains types and tables for managing operator assignments to deposits.
//! Assignments link specific deposits to operators who are responsible for processing
//! withdrawal requests within specified deadlines.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::bridge::{BitcoinBlockHeight, OperatorIdx};

use super::withdrawal::WithdrawalCommand;

/// Assignment entry linking a deposit to an operator for withdrawal processing.
///
/// Each assignment represents a task assigned to a specific operator to process
/// a withdrawal from a particular deposit. The assignment includes:
///
/// - **`deposit_idx`** - Reference to the deposit being processed
/// - **`withdrawal_cmd`** - Specification of outputs and amounts for withdrawal
/// - **`current_assignee`** - Operator currently responsible for executing the withdrawal
/// - **`previous_assignees`** - List of operators who were previously assigned but failed to execute
/// - **`exec_deadline`** - Bitcoin block height deadline for execution
///
/// # Execution Deadline
///
/// The `exec_deadline` represents the latest Bitcoin block height at which
/// the withdrawal can be executed. If a checkpoint is processed after this
/// height and the withdrawal hasn't been completed, the assignment becomes invalid.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct AssignmentEntry {
    deposit_idx: u32,

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

impl AssignmentEntry {
    /// Creates a new assignment entry.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - Index of the deposit to be processed
    /// - `withdrawal_cmd` - Withdrawal command with output specifications
    /// - `current_assignee` - Index of the operator assigned to execute the withdrawal
    /// - `exec_deadline` - Bitcoin block height deadline for execution
    ///
    /// # Returns
    ///
    /// A new [`AssignmentEntry`] instance.
    pub fn new(
        deposit_idx: u32,
        withdrawal_cmd: WithdrawalCommand,
        current_assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> Self {
        Self {
            deposit_idx,
            withdrawal_cmd,
            current_assignee,
            previous_assignees: Vec::new(),
            exec_deadline,
        }
    }

    /// Returns the deposit index associated with this assignment.
    ///
    /// # Returns
    ///
    /// The deposit index as [`u32`].
    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
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

    /// Returns a mutable reference to the list of previous assignees.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`Vec<OperatorIdx>`] of operators previously assigned to this withdrawal.
    pub fn previous_assignees_mut(&mut self) -> &mut Vec<OperatorIdx> {
        &mut self.previous_assignees
    }

    /// Returns the execution deadline for this assignment.
    ///
    /// # Returns
    ///
    /// The [`BitcoinBlockHeight`] deadline for withdrawal execution.
    pub fn exec_deadline(&self) -> BitcoinBlockHeight {
        self.exec_deadline
    }

    /// Updates the assigned operator for this assignment.
    ///
    /// # Parameters
    ///
    /// - `new_assignee` - New operator index to assign
    pub fn set_current_assignee(&mut self, new_assignee: OperatorIdx) {
        self.current_assignee = new_assignee;
    }

    /// Reassigns the withdrawal to a new operator, moving the current assignee to previous assignees.
    ///
    /// # Parameters
    ///
    /// - `new_assignee` - New operator index to assign
    pub fn reassign(&mut self, new_assignee: OperatorIdx) {
        self.previous_assignees.push(self.current_assignee);
        self.current_assignee = new_assignee;
    }

    /// Updates the execution deadline for this assignment.
    ///
    /// # Parameters
    ///
    /// - `exec_deadline` - New deadline block height
    pub fn set_exec_deadline(&mut self, exec_deadline: BitcoinBlockHeight) {
        self.exec_deadline = exec_deadline;
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

    /// Validates the assignment table's internal invariants.
    ///
    /// Ensures that the assignments vector is sorted by deposit index.
    ///
    /// # Panics
    ///
    /// Panics if the sorting invariant is violated, indicating a bug in the table implementation.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.assignments.is_sorted_by_key(|e| e.deposit_idx) {
            panic!("bridge_state: assignments list not sorted");
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

    /// Finds the position where an assignment with the given deposit index exists or should be
    /// inserted.
    ///
    /// Uses binary search to efficiently locate the position.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Ok(position)` if an assignment with this deposit index exists
    /// - `Err(position)` where the assignment should be inserted to maintain sort order
    pub fn get_assignment_entry_pos(&self, deposit_idx: u32) -> Result<u32, u32> {
        self.assignments
            .binary_search_by_key(&deposit_idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
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
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|i| &self.assignments[i as usize])
    }

    /// Retrieves a mutable reference to an assignment entry by its deposit index.
    ///
    /// Uses binary search for O(log n) lookup performance.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&mut AssignmentEntry)` if the assignment exists
    /// - `None` if no assignment for the given deposit index is found
    pub fn get_assignment_mut(&mut self, deposit_idx: u32) -> Option<&mut AssignmentEntry> {
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|i| &mut self.assignments[i as usize])
    }

    /// Retrieves an assignment entry by its position in the internal vector.
    ///
    /// This method accesses assignments by their storage position rather than their
    /// logical deposit index. Useful for iteration or when the position is known.
    ///
    /// # Parameters
    ///
    /// - `pos` - The position in the internal vector (0-based)
    ///
    /// # Returns
    ///
    /// - `Some(&AssignmentEntry)` if the position is valid
    /// - `None` if the position is out of bounds
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&AssignmentEntry> {
        self.assignments.get(pos as usize)
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
    pub fn insert(
        &mut self,
        deposit_idx: u32,
        withdrawal_cmd: WithdrawalCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) {
        let entry = AssignmentEntry::new(deposit_idx, withdrawal_cmd, assignee, exec_deadline);

        // Fast path: if this is larger than the last entry, just push
        if let Some(last) = self.assignments.last() {
            if deposit_idx > last.deposit_idx {
                self.assignments.push(entry);
                return;
            }
        } else {
            // Empty table, just push
            self.assignments.push(entry);
            return;
        }

        // Slow path: find the correct position and insert
        match self.get_assignment_entry_pos(deposit_idx) {
            Ok(_) => {
                panic!("Assignment with deposit_idx {deposit_idx} already exists");
            }
            Err(pos) => {
                self.assignments.insert(pos as usize, entry);
            }
        }
    }

    /// Attempts to create an assignment entry for a specific deposit index.
    ///
    /// This method is similar to [`insert`] but returns a boolean indicating
    /// success instead of panicking on duplicate indices.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - Deposit index for the assignment
    /// - `withdrawal_cmd` - Withdrawal command with output specifications
    /// - `assignee` - Operator index assigned to execute the withdrawal
    /// - `exec_deadline` - Bitcoin block height deadline for execution
    ///
    /// # Returns
    ///
    /// - `true` if the assignment was successfully created
    /// - `false` if an assignment with this deposit index already exists
    pub fn try_create_assignment(
        &mut self,
        deposit_idx: u32,
        withdrawal_cmd: WithdrawalCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> bool {
        match self.get_assignment_entry_pos(deposit_idx) {
            Ok(_) => false, // Assignment already exists
            Err(pos) => {
                let entry =
                    AssignmentEntry::new(deposit_idx, withdrawal_cmd, assignee, exec_deadline);
                self.assignments.insert(pos as usize, entry);
                true
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
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|pos| self.assignments.remove(pos as usize))
    }

    /// Returns an iterator over all deposit indices that have assignments.
    ///
    /// The indices are returned in sorted order due to the table's invariant.
    ///
    /// # Returns
    ///
    /// Iterator yielding deposit indices for all assignments.
    pub fn get_all_deposit_indices(&self) -> impl Iterator<Item = u32> + '_ {
        self.assignments.iter().map(|e| e.deposit_idx)
    }

    /// Returns an iterator over assignments for a specific operator.
    ///
    /// Filters all assignments to return only those assigned to the specified operator.
    ///
    /// # Parameters
    ///
    /// - `operator_idx` - The operator index to filter by
    ///
    /// # Returns
    ///
    /// Iterator yielding assignment entries assigned to the specified operator.
    pub fn get_assignments_by_operator(
        &self,
        operator_idx: OperatorIdx,
    ) -> impl Iterator<Item = &AssignmentEntry> + '_ {
        self.assignments
            .iter()
            .filter(move |e| e.current_assignee == operator_idx)
    }

    /// Returns an iterator over assignments that have expired.
    ///
    /// An assignment is considered expired if the current Bitcoin block height
    /// is greater than or equal to its execution deadline.
    ///
    /// # Parameters
    ///
    /// - `current_height` - The current Bitcoin block height for comparison
    ///
    /// # Returns
    ///
    /// Iterator yielding expired assignment entries.
    pub fn get_expired_assignments(
        &self,
        current_height: BitcoinBlockHeight,
    ) -> impl Iterator<Item = &AssignmentEntry> + '_ {
        self.assignments
            .iter()
            .filter(move |e| e.exec_deadline <= current_height)
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{bitcoin_bosd::Descriptor, l1::BitcoinAmount};

    use super::*;
    use crate::state::withdrawal::{WithdrawOutput, WithdrawalCommand};

    fn create_test_descriptor() -> Descriptor {
        // Create a simple test descriptor - this is just for testing
        Descriptor::new_p2pkh(&[0u8; 20])
    }

    fn create_test_withdrawal_command() -> WithdrawalCommand {
        let output = WithdrawOutput::new(create_test_descriptor(), BitcoinAmount::from_sat(1000));
        WithdrawalCommand::new(vec![output])
    }

    #[test]
    fn test_assignment_entry_new() {
        let withdrawal_cmd = create_test_withdrawal_command();
        let assignee = 5;
        let exec_deadline = 1000;

        let assignment = AssignmentEntry::new(10, withdrawal_cmd.clone(), assignee, exec_deadline);

        assert_eq!(assignment.deposit_idx(), 10);
        assert_eq!(assignment.current_assignee(), assignee);
        assert_eq!(assignment.exec_deadline(), exec_deadline);
        assert_eq!(assignment.withdrawal_command().withdraw_outputs().len(), 1);
    }

    #[test]
    fn test_assignment_entry_setters() {
        let withdrawal_cmd = create_test_withdrawal_command();
        let mut assignment = AssignmentEntry::new(10, withdrawal_cmd, 5, 1000);

        // Test assignee setter
        assignment.set_current_assignee(7);
        assert_eq!(assignment.current_assignee(), 7);

        // Test deadline setter
        assignment.set_exec_deadline(2000);
        assert_eq!(assignment.exec_deadline(), 2000);
    }

    #[test]
    fn test_assignment_table_new_empty() {
        let table = AssignmentTable::new_empty();

        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
        assert_eq!(table.assignments().len(), 0);
    }

    #[test]
    fn test_assignment_table_insert_sequential() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd1 = create_test_withdrawal_command();
        let withdrawal_cmd2 = create_test_withdrawal_command();

        // Insert first assignment
        table.insert(0, withdrawal_cmd1, 1, 1000);
        assert_eq!(table.len(), 1);

        let assignment = table.get_assignment(0).unwrap();
        assert_eq!(assignment.deposit_idx(), 0);
        assert_eq!(assignment.current_assignee(), 1);

        // Insert second assignment (sequential)
        table.insert(1, withdrawal_cmd2, 2, 2000);
        assert_eq!(table.len(), 2);

        let assignment2 = table.get_assignment(1).unwrap();
        assert_eq!(assignment2.deposit_idx(), 1);
        assert_eq!(assignment2.current_assignee(), 2);
    }

    #[test]
    fn test_assignment_table_insert_out_of_order() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        // Insert assignments out of order
        table.insert(5, withdrawal_cmd.clone(), 1, 1000);
        table.insert(2, withdrawal_cmd.clone(), 2, 2000);
        table.insert(7, withdrawal_cmd, 3, 3000);

        assert_eq!(table.len(), 3);

        // Check they are sorted by deposit_idx
        let assignments: Vec<_> = table.assignments().iter().collect();
        assert_eq!(assignments[0].deposit_idx(), 2);
        assert_eq!(assignments[1].deposit_idx(), 5);
        assert_eq!(assignments[2].deposit_idx(), 7);
    }

    #[test]
    #[should_panic(expected = "Assignment with deposit_idx 5 already exists")]
    fn test_assignment_table_insert_duplicate_panics() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(5, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd, 2, 2000); // Should panic
    }

    #[test]
    fn test_assignment_table_try_create_assignment() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        // Create first assignment
        let success = table.try_create_assignment(0, withdrawal_cmd.clone(), 1, 1000);
        assert!(success);
        assert_eq!(table.len(), 1);

        // Try to create duplicate (should fail)
        let success = table.try_create_assignment(0, withdrawal_cmd.clone(), 2, 2000);
        assert!(!success);
        assert_eq!(table.len(), 1);

        // Create new assignment
        let success = table.try_create_assignment(5, withdrawal_cmd, 2, 2000);
        assert!(success);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_assignment_table_get_assignment() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd, 2, 2000);

        // Test existing assignments
        assert!(table.get_assignment(0).is_some());
        assert!(table.get_assignment(5).is_some());

        // Test non-existing assignments
        assert!(table.get_assignment(1).is_none());
        assert!(table.get_assignment(100).is_none());
    }

    #[test]
    fn test_assignment_table_get_assignment_mut() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd, 1, 1000);

        // Test mutable access
        let assignment = table.get_assignment_mut(0).unwrap();
        assignment.set_current_assignee(5);

        // Verify the change
        let assignment = table.get_assignment(0).unwrap();
        assert_eq!(assignment.current_assignee(), 5);

        // Test non-existing assignment
        assert!(table.get_assignment_mut(100).is_none());
    }

    #[test]
    fn test_assignment_table_remove_assignment() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd, 2, 2000);

        assert_eq!(table.len(), 2);

        // Remove existing assignment
        let removed = table.remove_assignment(0);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().deposit_idx(), 0);
        assert_eq!(table.len(), 1);

        // Try to remove non-existing assignment
        let removed = table.remove_assignment(100);
        assert!(removed.is_none());
        assert_eq!(table.len(), 1);

        // Verify remaining assignment
        assert!(table.get_assignment(5).is_some());
        assert!(table.get_assignment(0).is_none());
    }

    #[test]
    fn test_assignment_table_get_entry_at_pos() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(2, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd, 2, 2000);

        // Test valid positions
        let assignment0 = table.get_entry_at_pos(0).unwrap();
        assert_eq!(assignment0.deposit_idx(), 2);

        let assignment1 = table.get_entry_at_pos(1).unwrap();
        assert_eq!(assignment1.deposit_idx(), 5);

        // Test invalid positions
        assert!(table.get_entry_at_pos(2).is_none());
        assert!(table.get_entry_at_pos(100).is_none());
    }

    #[test]
    fn test_assignment_table_get_all_deposit_indices() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd.clone(), 2, 2000);
        table.insert(2, withdrawal_cmd, 3, 3000);

        let indices: Vec<_> = table.get_all_deposit_indices().collect();
        assert_eq!(indices, vec![0, 2, 5]); // Should be sorted
    }

    #[test]
    fn test_assignment_table_get_assignments_by_operator() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(1, withdrawal_cmd.clone(), 2, 2000);
        table.insert(2, withdrawal_cmd.clone(), 1, 3000);
        table.insert(3, withdrawal_cmd, 3, 4000);

        // Get assignments for operator 1
        let op1_assignments: Vec<_> = table.get_assignments_by_operator(1).collect();
        assert_eq!(op1_assignments.len(), 2);
        assert_eq!(op1_assignments[0].deposit_idx(), 0);
        assert_eq!(op1_assignments[1].deposit_idx(), 2);

        // Get assignments for operator 2
        let op2_assignments: Vec<_> = table.get_assignments_by_operator(2).collect();
        assert_eq!(op2_assignments.len(), 1);
        assert_eq!(op2_assignments[0].deposit_idx(), 1);

        // Get assignments for non-existing operator
        let op99_assignments: Vec<_> = table.get_assignments_by_operator(99).collect();
        assert_eq!(op99_assignments.len(), 0);
    }

    #[test]
    fn test_assignment_table_get_expired_assignments() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(1, withdrawal_cmd.clone(), 2, 2000);
        table.insert(2, withdrawal_cmd.clone(), 3, 1500);
        table.insert(3, withdrawal_cmd, 4, 500);

        // Get expired assignments at height 1500
        let expired: Vec<_> = table.get_expired_assignments(1500).collect();
        assert_eq!(expired.len(), 3); // Heights 500, 1000, and 1500

        let expired_indices: Vec<_> = expired.iter().map(|a| a.deposit_idx()).collect();
        assert_eq!(expired_indices, vec![0, 2, 3]);

        // Get expired assignments at height 1000
        let expired: Vec<_> = table.get_expired_assignments(1000).collect();
        assert_eq!(expired.len(), 2); // Heights 500 and 1000

        // Get expired assignments at height 100 (should be none)
        let expired: Vec<_> = table.get_expired_assignments(100).collect();
        assert_eq!(expired.len(), 0);
    }

    #[test]
    fn test_assignment_table_get_assignment_entry_pos() {
        let mut table = AssignmentTable::new_empty();

        let withdrawal_cmd = create_test_withdrawal_command();

        table.insert(0, withdrawal_cmd.clone(), 1, 1000);
        table.insert(5, withdrawal_cmd, 2, 2000);

        // Test existing indices
        assert_eq!(table.get_assignment_entry_pos(0), Ok(0));
        assert_eq!(table.get_assignment_entry_pos(5), Ok(1));

        // Test non-existing indices (returns where it would be inserted)
        assert_eq!(table.get_assignment_entry_pos(3), Err(1));
        assert_eq!(table.get_assignment_entry_pos(10), Err(2));
    }
}
