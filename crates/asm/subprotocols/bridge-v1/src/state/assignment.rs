//! Operator Assignment Management
//!
//! This module contains types and tables for managing operator assignments to deposits.
//! Assignments link specific deposit UTXOs to operators who are responsible for processing
//! the corresponding withdrawal requests within specified deadlines.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use rand_chacha::{
    ChaChaRng,
    rand_core::{RngCore, SeedableRng},
};
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, BitcoinBlockHeight, BitcoinTxid, L1BlockId},
    operator::OperatorIdx,
    sorted_vec::SortedVec,
};

use super::withdrawal::WithdrawalCommand;
use crate::{
    errors::WithdrawalCommandError,
    state::{deposit::DepositEntry, operator::OperatorBitmap},
};

/// Assignment entry linking a deposit UTXO to an operator for withdrawal processing.
///
/// Each assignment represents a task, assigned to a specific operator to process
/// a withdrawal of from a particular deposit UTXO.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Arbitrary)]
pub struct AssignmentEntry {
    /// Deposit entry that has been assigned
    deposit_entry: DepositEntry,

    /// Withdrawal command specifying outputs and amounts.
    withdrawal_cmd: WithdrawalCommand,

    /// Index of the operator currently assigned to execute this withdrawal.
    ///
    /// If they successfully front the withdrawal based on `withdrawal_cmd`
    /// within the `exec_deadline`, they are able to unlock their claim.
    current_assignee: OperatorIdx,

    /// Bitmap of operators who were previously assigned to this withdrawal.
    ///
    /// When a withdrawal is reassigned, the current assignee is marked in this
    /// bitmap before a new operator is selected. This prevents reassigning to
    /// operators who have already failed to execute the withdrawal.
    previous_assignees: OperatorBitmap,

    /// Bitcoin block height deadline for withdrawal execution.
    ///
    /// The withdrawal fulfillment transaction must be executed before this block height for the
    /// operator to be eligible for [`ClaimUnlock`](super::withdrawal::OperatorClaimUnlock).
    exec_deadline: BitcoinBlockHeight,
}

impl PartialOrd for AssignmentEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AssignmentEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deposit_entry.cmp(&other.deposit_entry)
    }
}

/// Filters and returns eligible operators for assignment or reassignment.
///
/// Returns a bitmap of operators who meet all eligibility criteria:
/// - Must be part of the deposit's notary operator set
/// - Must not have previously been assigned to this withdrawal (prevents reassignment to failed
///   operators)
/// - Must be currently active in the network
///
/// # Parameters
///
/// - `notary_operators` - Bitmap of notary operators authorized for this deposit
/// - `previous_assignees` - Bitmap of operators who have previously been assigned but failed
/// - `current_active_operators` - Bitmap of operators currently active in the network
///
/// # Returns
///
/// [`OperatorBitmap`] representing eligible operators for assignment.
/// Returns empty bitmap if no operators meet all criteria.
fn filter_eligible_operators(
    notary_operators: &OperatorBitmap,
    previous_assignees: &OperatorBitmap,
    current_active_operators: &OperatorBitmap,
) -> OperatorBitmap {
    // Use bitwise operations for efficiency:
    // 1. Remove previous assignees from notary operators: notary_operators & (!previous_assignees)
    // 2. Keep only currently active operators: result & current_active_operators
    notary_operators
        .bitwise_and_not(previous_assignees)
        .bitwise_and(current_active_operators)
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
    /// - `current_active_operators` - Bitmap of currently active operator indices
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
        current_active_operators: &OperatorBitmap,
        seed: L1BlockId,
    ) -> Result<Self, WithdrawalCommandError> {
        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        let empty_bitmap = OperatorBitmap::new_empty(); // No previous assignees at creation

        let eligible_operators = filter_eligible_operators(
            deposit_entry.operators(),
            &empty_bitmap,
            current_active_operators,
        );

        if eligible_operators.is_empty() {
            return Err(WithdrawalCommandError::NoEligibleOperators {
                deposit_idx: deposit_entry.idx(),
            });
        }

        // Select a random operator from eligible ones
        let eligible_indices: Vec<OperatorIdx> = eligible_operators.to_indices();
        let random_index = (rng.next_u32() as usize) % eligible_indices.len();
        let current_assignee = eligible_indices[random_index];

        Ok(Self {
            deposit_entry,
            withdrawal_cmd,
            current_assignee,
            previous_assignees: OperatorBitmap::new_empty(),
            exec_deadline,
        })
    }

    /// Returns the deposit index associated with this assignment.
    pub fn deposit_idx(&self) -> u32 {
        self.deposit_entry.idx()
    }

    /// Returns the deposit txid associated with this assignment.
    pub fn deposit_txid(&self) -> BitcoinTxid {
        self.deposit_entry.output().outpoint().txid.into()
    }

    /// Returns a reference to the withdrawal command.
    pub fn withdrawal_command(&self) -> &WithdrawalCommand {
        &self.withdrawal_cmd
    }

    /// Returns the index of the currently assigned operator.
    pub fn current_assignee(&self) -> OperatorIdx {
        self.current_assignee
    }

    /// Returns a reference to the list of previous assignees.
    pub fn previous_assignees(&self) -> Vec<OperatorIdx> {
        self.previous_assignees.to_indices()
    }

    /// Returns the execution deadline for this assignment.
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
        new_operator_fee: BitcoinAmount,
        seed: L1BlockId,
        current_active_operators: &OperatorBitmap,
    ) -> Result<(), WithdrawalCommandError> {
        let _ = self.previous_assignees.try_set(self.current_assignee, true);

        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        // Convert notary operators to bitmap for efficient operations
        let notary_indices = self.deposit_entry.notary_operators();
        let mut notary_bitmap = OperatorBitmap::new_empty();
        for &idx in &notary_indices {
            let _ = notary_bitmap.try_set(idx, true);
        }

        let mut eligible_operators = filter_eligible_operators(
            &notary_bitmap,
            &self.previous_assignees,
            current_active_operators,
        );

        if eligible_operators.is_empty() {
            // If no eligible operators left, clear previous assignees
            self.previous_assignees = OperatorBitmap::new_empty();
            eligible_operators = filter_eligible_operators(
                &notary_bitmap,
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

        // Select a random operator from eligible ones
        let eligible_indices: Vec<OperatorIdx> = eligible_operators.to_indices();
        let random_index = (rng.next_u32() as usize) % eligible_indices.len();
        let new_assignee = eligible_indices[random_index];

        self.current_assignee = new_assignee;
        self.withdrawal_cmd.update_fee(new_operator_fee);
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
    assignments: SortedVec<AssignmentEntry>,
}

impl AssignmentTable {
    /// Creates a new empty assignment table with no assignments
    pub fn new_empty() -> Self {
        Self {
            assignments: SortedVec::new_empty(),
        }
    }

    /// Returns the number of assignments in the table.
    pub fn len(&self) -> u32 {
        self.assignments.len() as u32
    }

    /// Returns whether the assignment table is empty.
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// Returns a slice of all assignment entries.
    pub fn assignments(&self) -> &[AssignmentEntry] {
        self.assignments.as_slice()
    }

    /// Retrieves an assignment entry by its deposit index.
    /// # Returns
    ///
    /// - `Some(&AssignmentEntry)` if the assignment exists
    /// - `None` if no assignment for the given deposit index is found
    pub fn get_assignment(&self, deposit_idx: u32) -> Option<&AssignmentEntry> {
        self.assignments
            .as_slice()
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx())
            .ok()
            .map(|i| &self.assignments.as_slice()[i])
    }

    /// Creates a new assignment entry with optimized insertion.
    ///
    /// # Panics
    ///
    /// Panics if an assignment with the given deposit index already exists.
    pub fn insert(&mut self, entry: AssignmentEntry) {
        // Check if entry already exists
        if self.get_assignment(entry.deposit_idx()).is_some() {
            panic!(
                "Assignment with deposit index {} already exists",
                entry.deposit_idx()
            );
        }

        // SortedVec handles the insertion and maintains order
        self.assignments.insert(entry);
    }

    /// Removes an assignment by its deposit index.
    ///
    /// # Returns
    ///
    /// - `Some(AssignmentEntry)` if the assignment was found and removed
    /// - `None` if no assignment with the given deposit index exists
    pub fn remove_assignment(&mut self, deposit_idx: u32) -> Option<AssignmentEntry> {
        // Find the assignment first
        let assignment = self.get_assignment(deposit_idx)?.clone();

        // Remove it using SortedVec's remove method
        if self.assignments.remove(&assignment) {
            Some(assignment)
        } else {
            None
        }
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
    /// - `current_active_operators` - Bitmap of currently active operator indices
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
    /// let active_operators = OperatorBitmap::new_sequential_active(3);
    /// let seed = L1BlockId::from([0u8; 32]);
    ///
    /// table.reassign_expired_assignments(current_height, &active_operators, seed)?;
    /// ```
    pub fn reassign_expired_assignments(
        &mut self,
        operator_fee: BitcoinAmount,
        current_height: BitcoinBlockHeight,
        current_active_operators: &OperatorBitmap,
        seed: L1BlockId,
    ) -> Result<Vec<u32>, WithdrawalCommandError> {
        let mut reassigned_withdrawals = Vec::new();

        // Using iter_mut since we're only modifying non-sorting fields
        for assignment in self
            .assignments
            .iter_mut()
            .filter(|e| e.exec_deadline <= current_height)
        {
            assignment.reassign(operator_fee, seed, current_active_operators)?;
            reassigned_withdrawals.push(assignment.deposit_idx());
        }

        Ok(reassigned_withdrawals)
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{
        l1::{BitcoinBlockHeight, L1BlockId},
        operator::OperatorIdx,
    };
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
        let current_active_operators = {
            let notary_indices = deposit_entry.notary_operators();
            let mut bitmap = OperatorBitmap::new_empty();
            for &idx in &notary_indices {
                let _ = bitmap.try_set(idx, true);
            }
            bitmap
        };

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
        assert!(current_active_operators.is_active(assignment.current_assignee()));
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
        let current_active_operators = OperatorBitmap::new_empty();

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
        let current_active_operators = {
            let notary_indices = deposit_entry.notary_operators();
            let mut bitmap = OperatorBitmap::new_empty();
            for &idx in &notary_indices {
                let _ = bitmap.try_set(idx, true);
            }
            bitmap
        };
        let new_fee = BitcoinAmount::from_sat(20_000);

        // Ensure we have at least 2 operators for reassignment
        if current_active_operators.active_count() < 2 {
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
        let result = assignment.reassign(new_fee, seed2, &current_active_operators);
        assert!(result.is_ok());

        // Verify reassignment - the behavior depends on how many operators are available
        if assignment.previous_assignees().len() == 1 {
            // Normal case: different operator selected and previous assignee tracked
            assert_eq!(assignment.previous_assignees()[0], original_assignee);
            assert_ne!(assignment.current_assignee(), original_assignee);
        } else {
            // Edge case: previous assignees were cleared during reassignment
            // This happens when no eligible operators are found initially, forcing
            // the reassignment logic to clear previous assignees and retry
            assert_eq!(assignment.previous_assignees().len(), 0);
        }
        assert!(current_active_operators.is_active(assignment.current_assignee()));
    }

    #[test]
    fn test_reassign_all_operators_exhausted() {
        let mut arb = ArbitraryGenerator::new();
        let mut deposit_entry: DepositEntry = arb.generate();

        // Force single operator for this test
        let operators = OperatorBitmap::new_sequential_active(1);
        deposit_entry = DepositEntry::new(
            deposit_entry.idx(),
            *deposit_entry.output(),
            operators,
            deposit_entry.amt(),
        )
        .unwrap();

        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let new_operator_fee: BitcoinAmount = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed1: L1BlockId = arb.generate();
        let seed2: L1BlockId = arb.generate();

        let current_active_operators = OperatorBitmap::new_sequential_active(1); // Single operator with index 0

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        // First reassignment should work (clears previous assignees and reassigns to same operator)
        let result = assignment.reassign(new_operator_fee, seed2, &current_active_operators);
        assert!(result.is_ok());

        // Should have cleared previous assignees and reassigned to the same operator
        assert_eq!(assignment.previous_assignees().len(), 0);
        assert_eq!(assignment.current_assignee(), 0); // Should be operator index 0
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
        let current_active_operators = {
            let notary_indices = deposit_entry.notary_operators();
            let mut bitmap = OperatorBitmap::new_empty();
            for &idx in &notary_indices {
                let _ = bitmap.try_set(idx, true);
            }
            bitmap
        };

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
        let new_operator_fee: BitcoinAmount = arb.generate();

        // Create expired assignment (deadline < current_height)
        let deposit_entry1: DepositEntry = arb.generate();
        let withdrawal_cmd1: WithdrawalCommand = arb.generate();
        let expired_deadline: BitcoinBlockHeight = 100; // Less than current_height
        let current_active_operators1 = {
            let notary_indices = deposit_entry1.notary_operators();
            let mut bitmap = OperatorBitmap::new_empty();
            for &idx in &notary_indices {
                let _ = bitmap.try_set(idx, true);
            }
            bitmap
        };

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
        let current_active_operators2 = {
            let notary_indices = deposit_entry2.notary_operators();
            let mut bitmap = OperatorBitmap::new_empty();
            for &idx in &notary_indices {
                let _ = bitmap.try_set(idx, true);
            }
            bitmap
        };

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

        // Create combined active operators bitmap for reassignment
        let mut combined_active_operators = OperatorBitmap::new_empty();
        for idx in current_active_operators1.active_indices() {
            let _ = combined_active_operators.try_set(idx, true);
        }
        for idx in current_active_operators2.active_indices() {
            let _ = combined_active_operators.try_set(idx, true);
        }

        // Reassign expired assignments
        let result = table.reassign_expired_assignments(
            new_operator_fee,
            current_height,
            &combined_active_operators,
            seed,
        );

        assert!(result.is_ok(), "Reassignment should succeed");

        // Check that expired assignment was reassigned
        let expired_assignment_after = table.get_assignment(expired_deposit_idx).unwrap();

        // The behavior depends on how many eligible operators are available
        let deposit1_notary_count = deposit_entry1.notary_operators().len();
        if deposit1_notary_count > 1
            && expired_assignment_after.current_assignee() != original_assignee
        {
            // Normal case: different operator selected
            assert_eq!(expired_assignment_after.previous_assignees().len(), 1);
            assert_eq!(
                expired_assignment_after.previous_assignees()[0],
                original_assignee
            );
            assert_ne!(
                expired_assignment_after.current_assignee(),
                original_assignee
            );
        } else {
            // Edge case: same operator reselected or only one operator available
            // In this case previous assignees may be cleared to allow reassignment
            // This can happen with single operator deposits or when the same operator is reselected
            assert_eq!(expired_assignment_after.previous_assignees().len(), 0);
        }

        // Check that non-expired assignment was not reassigned
        let future_assignment_after = table.get_assignment(future_deposit_idx).unwrap();
        assert_eq!(future_assignment_after.previous_assignees().len(), 0);
        assert_eq!(
            future_assignment_after.current_assignee(),
            future_original_assignee
        );
    }
}
