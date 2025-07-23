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
                panic!("Assignment with deposit_idx {} already exists", deposit_idx);
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
