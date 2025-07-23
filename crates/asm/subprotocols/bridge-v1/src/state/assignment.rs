//! Deposit state types and state transitions.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::bridge::{BitcoinBlockHeight, OperatorIdx};

use super::withdrawal::WithdrawalCommand;

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct AssignmentEntry {
    deposit_idx: u32,

    /// Configuration for outputs to be written to.
    withdrawal_cmd: WithdrawalCommand,

    /// The index of the operator that's fronting the funds for the withdrawal,
    /// and who will be reimbursed by the bridge notaries.
    assignee: OperatorIdx,

    /// L1 block height before which we expect the dispatch command to be
    /// executed and after which this assignment command is no longer valid.
    ///
    /// If a checkpoint is processed for this L1 height and the withdrawal still
    /// goes out it won't be honored.
    exec_deadline: BitcoinBlockHeight,
}

impl AssignmentEntry {
    pub fn new(
        deposit_idx: u32,
        withdrawal_cmd: WithdrawalCommand,
        assignee: OperatorIdx,
        exec_deadline: BitcoinBlockHeight,
    ) -> Self {
        Self {
            deposit_idx,
            withdrawal_cmd,
            assignee,
            exec_deadline,
        }
    }

    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

    pub fn cmd(&self) -> &WithdrawalCommand {
        &self.withdrawal_cmd
    }

    pub fn assignee(&self) -> OperatorIdx {
        self.assignee
    }

    pub fn exec_deadline(&self) -> BitcoinBlockHeight {
        self.exec_deadline
    }

    pub fn set_assignee(&mut self, assignee_op_idx: OperatorIdx) {
        self.assignee = assignee_op_idx;
    }

    pub fn set_exec_deadline(&mut self, exec_deadline: BitcoinBlockHeight) {
        self.exec_deadline = exec_deadline;
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct AssignmentTable {
    /// Assignment table.
    ///
    /// MUST be sorted by `deposit_idx`.
    assignments: Vec<AssignmentEntry>,
}

impl AssignmentTable {
    pub fn new_empty() -> Self {
        Self {
            assignments: Vec::new(),
        }
    }

    /// Sanity checks the assignment table for sensibility.
    #[allow(dead_code)] // FIXME: remove this.
    fn sanity_check(&self) {
        if !self.assignments.is_sorted_by_key(|e| e.deposit_idx) {
            panic!("bridge_state: assignments list not sorted");
        }
    }

    /// Returns the number of assignment entries.
    pub fn len(&self) -> u32 {
        self.assignments.len() as u32
    }

    /// Returns if the assignment table is empty.
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    pub fn assignments(&self) -> &[AssignmentEntry] {
        &self.assignments
    }

    /// Gets the position in the assignment table of a hypothetical assignment entry
    /// by deposit index.
    pub fn get_assignment_entry_pos(&self, deposit_idx: u32) -> Result<u32, u32> {
        self.assignments
            .binary_search_by_key(&deposit_idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
    }

    /// Gets an assignment from the table by its deposit idx.
    ///
    /// Does a binary search.
    pub fn get_assignment(&self, deposit_idx: u32) -> Option<&AssignmentEntry> {
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|i| &self.assignments[i as usize])
    }

    /// Gets a mut ref to an assignment from the table by its deposit idx.
    ///
    /// Does a binary search.
    pub fn get_assignment_mut(&mut self, deposit_idx: u32) -> Option<&mut AssignmentEntry> {
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|i| &mut self.assignments[i as usize])
    }

    /// Gets an assignment entry by its internal position, *ignoring* the deposit indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&AssignmentEntry> {
        self.assignments.get(pos as usize)
    }

    /// Inserts a new assignment entry. Optimized for sequential insertion.
    ///
    /// If the deposit_idx is larger than the last entry, it will directly push.
    /// Otherwise, it performs a binary search to find the correct position.
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

    /// Tries to create an assignment entry for a specific deposit idx.
    ///
    /// Returns if we inserted it successfully.
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

    /// Removes an assignment by deposit idx.
    ///
    /// Returns the removed assignment if it existed.
    pub fn remove_assignment(&mut self, deposit_idx: u32) -> Option<AssignmentEntry> {
        self.get_assignment_entry_pos(deposit_idx)
            .ok()
            .map(|pos| self.assignments.remove(pos as usize))
    }

    /// Get all deposit indices that have assignments.
    pub fn get_all_deposit_indices(&self) -> impl Iterator<Item = u32> + '_ {
        self.assignments.iter().map(|e| e.deposit_idx)
    }

    /// Get assignments by assignee operator.
    pub fn get_assignments_by_operator(
        &self,
        operator_idx: OperatorIdx,
    ) -> impl Iterator<Item = &AssignmentEntry> + '_ {
        self.assignments
            .iter()
            .filter(move |e| e.assignee == operator_idx)
    }

    /// Get assignments that are expired (past their exec_deadline).
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
    use super::*;
    use crate::state::withdrawal::{WithdrawOutput, WithdrawalCommand};
    use strata_primitives::{
        bitcoin_bosd::Descriptor,
        buf::Buf32,
        l1::{BitcoinAmount, OutputRef},
    };

    fn create_test_output_ref(idx: u32) -> OutputRef {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[0] = idx as u8;
        OutputRef::new(Buf32::from(hash_bytes).into(), idx)
    }

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
        assert_eq!(assignment.assignee(), assignee);
        assert_eq!(assignment.exec_deadline(), exec_deadline);
        assert_eq!(assignment.cmd().withdraw_outputs().len(), 1);
    }

    #[test]
    fn test_assignment_entry_setters() {
        let withdrawal_cmd = create_test_withdrawal_command();
        let mut assignment = AssignmentEntry::new(10, withdrawal_cmd, 5, 1000);
        
        // Test assignee setter
        assignment.set_assignee(7);
        assert_eq!(assignment.assignee(), 7);
        
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
        assert_eq!(assignment.assignee(), 1);
        
        // Insert second assignment (sequential)
        table.insert(1, withdrawal_cmd2, 2, 2000);
        assert_eq!(table.len(), 2);
        
        let assignment2 = table.get_assignment(1).unwrap();
        assert_eq!(assignment2.deposit_idx(), 1);
        assert_eq!(assignment2.assignee(), 2);
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
        let assignments: Vec<_> = table.assignments().into_iter().collect();
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
        assignment.set_assignee(5);
        
        // Verify the change
        let assignment = table.get_assignment(0).unwrap();
        assert_eq!(assignment.assignee(), 5);
        
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
