//! Operator Assignment Management
//!
//! This module contains types and tables for managing operator assignments to deposits.
//! Assignments link specific deposits to operators who are responsible for processing
//! withdrawal requests within specified deadlines.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::{BitcoinBlockHeight, OperatorIdx},
    buf::Buf32,
};

use super::withdrawal::WithdrawalCommand;
use crate::state::deposit::DepositEntry;

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
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
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
        deposit_entry: DepositEntry,
        withdrawal_cmd: WithdrawalCommand,
        current_assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> Self {
        Self {
            deposit_entry,
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

    /// Returns a mutable reference to the list of previous assignees.
    ///
    /// # Returns
    ///
    /// Mutable reference to the [`Vec<OperatorIdx>`] of operators previously assigned to this
    /// withdrawal.
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

    /// Reassigns the withdrawal to a new operator, moving the current assignee to previous
    /// assignees.
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
        self.assignments
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx())
            .ok()
            .map(|i| &mut self.assignments[i])
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
        deposit_entry: DepositEntry,
        withdrawal_cmd: WithdrawalCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) {
        let idx = deposit_entry.idx();
        let entry = AssignmentEntry::new(deposit_entry, withdrawal_cmd, assignee, exec_deadline);

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

    /// Returns an iterator over all deposit indices that have assignments.
    ///
    /// The indices are returned in sorted order due to the table's invariant.
    ///
    /// # Returns
    ///
    /// Iterator yielding deposit indices for all assignments.
    pub fn get_all_deposit_indices(&self) -> impl Iterator<Item = u32> + '_ {
        self.assignments.iter().map(|e| e.deposit_idx())
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
    #[cfg(not(feature = "test_utils"))]
    use bitcoin::{Txid, hashes::Hash};
    use strata_primitives::{
        bitcoin_bosd::Descriptor,
        bridge::OperatorIdx,
        l1::{BitcoinAmount, OutputRef},
    };

    use super::*;
    use crate::state::withdrawal::WithdrawOutput;

    #[cfg(feature = "test_utils")]
    use strata_test_utils::ArbitraryGenerator;

    fn create_test_deposit_entry(idx: u32) -> DepositEntry {
        #[cfg(feature = "test_utils")]
        {
            // Use the Arbitrary implementation when available
            let mut generator = ArbitraryGenerator::new();
            let deposit = generator.generate::<DepositEntry>();
            
            // Override the index to ensure it matches our test needs
            let output_ref = OutputRef::new(
                deposit.output().outpoint().txid,
                deposit.output().outpoint().vout,
            );
            
            DepositEntry::new(
                idx,
                output_ref,
                deposit.notary_operators().to_vec(),
                deposit.amt(),
            ).unwrap()
        }
        #[cfg(not(feature = "test_utils"))]
        {
            // Fallback implementation when test_utils feature is not enabled
            let txid = Txid::from_byte_array([idx as u8; 32]);
            let output_ref = OutputRef::new(txid, 0);
            let operators = vec![1, 2, 3]; // Test with 3 operators
            let amount = BitcoinAmount::from_sat(1000000); // 0.01 BTC

            DepositEntry::new(idx, output_ref, operators, amount).unwrap()
        }
    }

    fn create_test_withdrawal_command() -> WithdrawalCommand {
        // Create a simple P2PKH descriptor for testing
        let pubkey_hash = [0u8; 20]; // dummy pubkey hash
        let destination = Descriptor::new_p2pkh(&pubkey_hash);
        let amount = BitcoinAmount::from_sat(500000); // 0.005 BTC
        let output = WithdrawOutput::new(destination, amount);
        WithdrawalCommand::new(vec![output])
    }

    mod assignment_entry_tests {
        use super::*;

        #[test]
        fn test_new_assignment_entry() {
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let deadline = 1000u64.into();

            let entry = AssignmentEntry::new(
                deposit_entry.clone(),
                withdrawal_cmd.clone(),
                assignee,
                deadline,
            );

            assert_eq!(entry.deposit_idx(), 42);
            assert_eq!(
                entry.deposit_txid(),
                deposit_entry.output().outpoint().txid.into()
            );
            assert_eq!(entry.withdrawal_command(), &withdrawal_cmd);
            assert_eq!(entry.current_assignee(), assignee);
            assert_eq!(entry.previous_assignees(), &[] as &[OperatorIdx]);
            assert_eq!(entry.exec_deadline(), deadline);
        }

        #[test]
        fn test_set_current_assignee() {
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let initial_assignee: OperatorIdx = 1;
            let new_assignee: OperatorIdx = 2;
            let deadline = 1000u64.into();

            let mut entry =
                AssignmentEntry::new(deposit_entry, withdrawal_cmd, initial_assignee, deadline);

            entry.set_current_assignee(new_assignee);
            assert_eq!(entry.current_assignee(), new_assignee);
            assert_eq!(entry.previous_assignees(), &[] as &[OperatorIdx]);
        }

        #[test]
        fn test_reassign() {
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let initial_assignee: OperatorIdx = 1;
            let new_assignee: OperatorIdx = 2;
            let deadline = 1000u64.into();

            let mut entry =
                AssignmentEntry::new(deposit_entry, withdrawal_cmd, initial_assignee, deadline);

            entry.reassign(new_assignee);
            assert_eq!(entry.current_assignee(), new_assignee);
            assert_eq!(entry.previous_assignees(), &[initial_assignee]);

            // Test multiple reassignments
            let third_assignee: OperatorIdx = 3;
            entry.reassign(third_assignee);
            assert_eq!(entry.current_assignee(), third_assignee);
            assert_eq!(
                entry.previous_assignees(),
                &[initial_assignee, new_assignee]
            );
        }

        #[test]
        fn test_set_exec_deadline() {
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let initial_deadline = 1000u64.into();
            let new_deadline = 2000u64.into();

            let mut entry =
                AssignmentEntry::new(deposit_entry, withdrawal_cmd, assignee, initial_deadline);

            entry.set_exec_deadline(new_deadline);
            assert_eq!(entry.exec_deadline(), new_deadline);
        }

        #[test]
        fn test_previous_assignees_mut() {
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let deadline = 1000u64.into();

            let mut entry = AssignmentEntry::new(deposit_entry, withdrawal_cmd, assignee, deadline);

            // Test direct manipulation of previous assignees
            entry.previous_assignees_mut().push(5);
            entry.previous_assignees_mut().push(6);

            assert_eq!(entry.previous_assignees(), &[5, 6]);
        }
    }

    mod assignment_table_tests {
        use super::*;

        #[test]
        fn test_new_empty_table() {
            let table = AssignmentTable::new_empty();
            assert_eq!(table.len(), 0);
            assert!(table.is_empty());
            assert_eq!(table.assignments(), &[] as &[AssignmentEntry]);
        }

        #[test]
        fn test_insert_single_assignment() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let deadline = 1000u64.into();

            table.insert(
                deposit_entry.clone(),
                withdrawal_cmd.clone(),
                assignee,
                deadline,
            );

            assert_eq!(table.len(), 1);
            assert!(!table.is_empty());

            let assignment = table.get_assignment(42).unwrap();
            assert_eq!(assignment.deposit_idx(), 42);
            assert_eq!(assignment.current_assignee(), assignee);
        }

        #[test]
        fn test_insert_multiple_assignments_ordered() {
            let mut table = AssignmentTable::new_empty();

            // Insert in ascending order (fast path)
            for i in [1, 5, 10, 15] {
                let deposit_entry = create_test_deposit_entry(i);
                let withdrawal_cmd = create_test_withdrawal_command();
                table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());
            }

            assert_eq!(table.len(), 4);
            let indices: Vec<u32> = table.get_all_deposit_indices().collect();
            assert_eq!(indices, vec![1, 5, 10, 15]);
        }

        #[test]
        fn test_insert_multiple_assignments_unordered() {
            let mut table = AssignmentTable::new_empty();

            // Insert in random order to test binary search insertion
            for i in [10, 2, 15, 5, 1] {
                let deposit_entry = create_test_deposit_entry(i);
                let withdrawal_cmd = create_test_withdrawal_command();
                table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());
            }

            assert_eq!(table.len(), 5);
            let indices: Vec<u32> = table.get_all_deposit_indices().collect();
            assert_eq!(indices, vec![1, 2, 5, 10, 15]);
        }

        #[test]
        #[should_panic]
        fn test_insert_duplicate_assignment() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();

            table.insert(
                deposit_entry.clone(),
                withdrawal_cmd.clone(),
                1,
                1000u64.into(),
            );
            // This should panic due to duplicate insertion
            table.insert(deposit_entry, withdrawal_cmd, 2, 2000u64.into());
        }

        #[test]
        fn test_get_assignment() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;

            table.insert(deposit_entry, withdrawal_cmd, assignee, 1000u64.into());

            // Test successful lookup
            let assignment = table.get_assignment(42);
            assert!(assignment.is_some());
            assert_eq!(assignment.unwrap().deposit_idx(), 42);

            // Test failed lookup
            let missing = table.get_assignment(99);
            assert!(missing.is_none());
        }

        #[test]
        fn test_get_assignment_mut() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let new_assignee: OperatorIdx = 2;

            table.insert(deposit_entry, withdrawal_cmd, assignee, 1000u64.into());

            // Test mutable access and modification
            {
                let assignment = table.get_assignment_mut(42);
                assert!(assignment.is_some());
                assignment.unwrap().set_current_assignee(new_assignee);
            }

            // Verify the change
            let assignment = table.get_assignment(42).unwrap();
            assert_eq!(assignment.current_assignee(), new_assignee);
        }

        #[test]
        fn test_remove_assignment() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;

            table.insert(deposit_entry, withdrawal_cmd, assignee, 1000u64.into());
            assert_eq!(table.len(), 1);

            // Test successful removal
            let removed = table.remove_assignment(42);
            assert!(removed.is_some());
            assert_eq!(removed.unwrap().deposit_idx(), 42);
            assert_eq!(table.len(), 0);
            assert!(table.is_empty());

            // Test removal of non-existent assignment
            let missing = table.remove_assignment(99);
            assert!(missing.is_none());
        }

        #[test]
        fn test_get_assignments_by_operator() {
            let mut table = AssignmentTable::new_empty();
            let operator1: OperatorIdx = 1;
            let operator2: OperatorIdx = 2;

            // Insert assignments for different operators
            for i in 1..=5 {
                let deposit_entry = create_test_deposit_entry(i);
                let withdrawal_cmd = create_test_withdrawal_command();
                let assignee = if i % 2 == 0 { operator2 } else { operator1 };
                table.insert(deposit_entry, withdrawal_cmd, assignee, 1000u64.into());
            }

            // Test filtering by operator1 (should get indices 1, 3, 5)
            let op1_assignments: Vec<u32> = table
                .get_assignments_by_operator(operator1)
                .map(|a| a.deposit_idx())
                .collect();
            assert_eq!(op1_assignments, vec![1, 3, 5]);

            // Test filtering by operator2 (should get indices 2, 4)
            let op2_assignments: Vec<u32> = table
                .get_assignments_by_operator(operator2)
                .map(|a| a.deposit_idx())
                .collect();
            assert_eq!(op2_assignments, vec![2, 4]);
        }

        #[test]
        fn test_get_expired_assignments() {
            let mut table = AssignmentTable::new_empty();

            // Insert assignments with different deadlines
            let deadlines = [500u64, 1000u64, 1500u64, 2000u64];
            for (i, &deadline) in deadlines.iter().enumerate() {
                let deposit_entry = create_test_deposit_entry(i as u32 + 1);
                let withdrawal_cmd = create_test_withdrawal_command();
                table.insert(deposit_entry, withdrawal_cmd, 1, deadline.into());
            }

            // Test with current height = 1200 (should expire assignments 1 and 2)
            let current_height = 1200u64.into();
            let expired: Vec<u32> = table
                .get_expired_assignments(current_height)
                .map(|a| a.deposit_idx())
                .collect();
            assert_eq!(expired, vec![1, 2]);

            // Test with current height = 500 (should expire assignment 1 only)
            let current_height = 500u64.into();
            let expired: Vec<u32> = table
                .get_expired_assignments(current_height)
                .map(|a| a.deposit_idx())
                .collect();
            assert_eq!(expired, vec![1]);

            // Test with current height = 100 (should expire nothing)
            let current_height = 100u64.into();
            let expired: Vec<u32> = table
                .get_expired_assignments(current_height)
                .map(|a| a.deposit_idx())
                .collect();
            assert!(expired.is_empty());
        }

        #[test]
        fn test_assignment_sorting_invariant() {
            let mut table = AssignmentTable::new_empty();

            // Insert assignments in random order
            let indices = [50, 10, 30, 20, 40, 60];
            for &idx in &indices {
                let deposit_entry = create_test_deposit_entry(idx);
                let withdrawal_cmd = create_test_withdrawal_command();
                table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());
            }

            // Verify they are stored in sorted order
            let stored_indices: Vec<u32> = table.get_all_deposit_indices().collect();
            let mut expected_indices = indices.to_vec();
            expected_indices.sort();
            assert_eq!(stored_indices, expected_indices);
        }
    }

    mod edge_case_tests {
        use super::*;

        #[test]
        fn test_assignment_table_empty_operations() {
            let mut table = AssignmentTable::new_empty();

            // Test operations on empty table
            assert!(table.get_assignment(42).is_none());
            assert!(table.get_assignment_mut(42).is_none());
            assert!(table.remove_assignment(42).is_none());

            let op1_assignments: Vec<_> = table.get_assignments_by_operator(1).collect();
            assert!(op1_assignments.is_empty());

            let expired: Vec<_> = table.get_expired_assignments(1000u64.into()).collect();
            assert!(expired.is_empty());

            let indices: Vec<_> = table.get_all_deposit_indices().collect();
            assert!(indices.is_empty());
        }

        #[test]
        fn test_assignment_table_single_element_operations() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(42);
            let withdrawal_cmd = create_test_withdrawal_command();
            let assignee: OperatorIdx = 1;
            let deadline = 1000u64.into();

            table.insert(deposit_entry, withdrawal_cmd, assignee, deadline);

            // Test operations on single-element table
            assert!(table.get_assignment(41).is_none());
            assert!(table.get_assignment(42).is_some());
            assert!(table.get_assignment(43).is_none());

            // Test filtering with no matches
            let op2_assignments: Vec<_> = table.get_assignments_by_operator(2).collect();
            assert!(op2_assignments.is_empty());

            // Test filtering with matches
            let op1_assignments: Vec<_> = table.get_assignments_by_operator(1).collect();
            assert_eq!(op1_assignments.len(), 1);
        }

        #[test]
        fn test_assignment_deadline_edge_cases() {
            let mut table = AssignmentTable::new_empty();
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let deadline = 1000u64.into();

            table.insert(deposit_entry, withdrawal_cmd, 1, deadline);

            // Test exact deadline match (should be expired)
            let expired: Vec<_> = table.get_expired_assignments(1000u64.into()).collect();
            assert_eq!(expired.len(), 1);

            // Test one block before deadline (should not be expired)
            let expired: Vec<_> = table.get_expired_assignments(999u64.into()).collect();
            assert!(expired.is_empty());

            // Test one block after deadline (should be expired)
            let expired: Vec<_> = table.get_expired_assignments(1001u64.into()).collect();
            assert_eq!(expired.len(), 1);
        }

        #[test]
        fn test_assignment_entry_with_many_reassignments() {
            let deposit_entry = create_test_deposit_entry(1);
            let withdrawal_cmd = create_test_withdrawal_command();
            let initial_assignee: OperatorIdx = 1;
            let deadline = 1000u64.into();

            let mut entry =
                AssignmentEntry::new(deposit_entry, withdrawal_cmd, initial_assignee, deadline);

            // Perform many reassignments
            for i in 2..=100 {
                entry.reassign(i);
            }

            assert_eq!(entry.current_assignee(), 100);
            assert_eq!(entry.previous_assignees().len(), 99);
            assert_eq!(entry.previous_assignees()[0], 1);
            assert_eq!(entry.previous_assignees()[98], 99);
        }

        #[test]
        fn test_assignment_table_large_scale_operations() {
            let mut table = AssignmentTable::new_empty();

            // Insert many assignments in reverse order to stress binary search
            for i in (1..=1000).rev() {
                let deposit_entry = create_test_deposit_entry(i);
                let withdrawal_cmd = create_test_withdrawal_command();
                let assignee: OperatorIdx = (i % 10) + 1;
                table.insert(deposit_entry, withdrawal_cmd, assignee, (i + 1000).into());
            }

            assert_eq!(table.len(), 1000);

            // Verify sorting invariant
            let indices: Vec<u32> = table.get_all_deposit_indices().collect();
            for i in 0..999 {
                assert!(indices[i] < indices[i + 1], "Sorting invariant violated");
            }

            // Test filtering operations work correctly on large dataset
            let op1_assignments: Vec<_> = table.get_assignments_by_operator(1).collect();
            assert_eq!(op1_assignments.len(), 100); // Every 10th assignment

            // Test removal from middle
            assert!(table.remove_assignment(500).is_some());
            assert_eq!(table.len(), 999);
            assert!(table.get_assignment(500).is_none());
        }

        #[test]
        fn test_assignment_table_boundary_insertions() {
            let mut table = AssignmentTable::new_empty();

            // Test insertions that trigger different code paths
            let deposit_entry = create_test_deposit_entry(100);
            let withdrawal_cmd = create_test_withdrawal_command();
            table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());

            // Insert at beginning (should use binary search)
            let deposit_entry = create_test_deposit_entry(50);
            let withdrawal_cmd = create_test_withdrawal_command();
            table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());

            // Insert at end (should use fast path)
            let deposit_entry = create_test_deposit_entry(150);
            let withdrawal_cmd = create_test_withdrawal_command();
            table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());

            // Insert in middle (should use binary search)
            let deposit_entry = create_test_deposit_entry(75);
            let withdrawal_cmd = create_test_withdrawal_command();
            table.insert(deposit_entry, withdrawal_cmd, 1, 1000u64.into());

            let indices: Vec<u32> = table.get_all_deposit_indices().collect();
            assert_eq!(indices, vec![50, 75, 100, 150]);
        }
    }
}
