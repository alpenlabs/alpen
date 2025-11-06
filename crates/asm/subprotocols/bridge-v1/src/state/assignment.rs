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
use strata_bridge_types::OperatorIdx;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, BitcoinBlockHeight, BitcoinTxid, L1BlockId},
    sorted_vec::SortedVec,
};

use super::withdrawal::WithdrawalCommand;
use crate::{
    errors::{WithdrawalAssignmentError, WithdrawalCommandError},
    state::{bitmap::OperatorBitmap, deposit::DepositEntry},
};

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
/// `Result<OperatorBitmap, WithdrawalAssignmentError>` - Either the filtered bitmap of eligible
/// operators, or an error if the input bitmaps have incompatible lengths.
///
/// # Errors
///
/// - [`WithdrawalAssignmentError::MismatchedBitmapLengths`] - If notary_operators and
///   previous_assignees have different lengths
/// - [`WithdrawalAssignmentError::InsufficientActiveBitmapLength`] - If current_active_operators is
///   shorter than notary_operators
fn filter_eligible_operators(
    notary_operators: &OperatorBitmap,
    previous_assignees: &OperatorBitmap,
    current_active_operators: &OperatorBitmap,
) -> Result<OperatorBitmap, WithdrawalAssignmentError> {
    // Notary operators and previous assignees must have the same length to ensure
    // bitwise operations don't panic
    if notary_operators.len() != previous_assignees.len() {
        return Err(WithdrawalAssignmentError::MismatchedBitmapLengths {
            notary_len: notary_operators.len(),
            previous_len: previous_assignees.len(),
        });
    }

    // If current_active_operators is shorter, this indicates a system inconsistency
    // since we only append operator indices to bitmaps, never remove them.
    // We also need to ensure sufficient length to avoid panics during bitwise operations.
    if current_active_operators.len() < notary_operators.len() {
        return Err(WithdrawalAssignmentError::InsufficientActiveBitmapLength {
            active_len: current_active_operators.len(),
            notary_len: notary_operators.len(),
        });
    }

    let notary_len = notary_operators.len();

    // Clone and truncate current_active_operators to match notary length
    let mut active_truncated = current_active_operators.bits.clone();
    active_truncated.truncate(notary_len);

    // In-place operations: active = (notary & !previous) & active
    active_truncated &= &notary_operators.bits;
    active_truncated &= &!previous_assignees.bits.clone();

    Ok(active_truncated.into())
}

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
    /// - `Err(WithdrawalAssignmentError)` - If no eligible operators are available or bitmap
    ///   operation fails
    pub fn create_with_random_assignment(
        deposit_entry: DepositEntry,
        withdrawal_cmd: WithdrawalCommand,
        exec_deadline: BitcoinBlockHeight,
        current_active_operators: &OperatorBitmap,
        seed: L1BlockId,
    ) -> Result<Self, WithdrawalAssignmentError> {
        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        // No previous assignees at creation
        let previous_assignees =
            OperatorBitmap::new_with_size(deposit_entry.notary_operators().len(), false);

        let eligible_operators = filter_eligible_operators(
            deposit_entry.notary_operators(),
            &previous_assignees,
            current_active_operators,
        )?;

        let active_count = eligible_operators.active_count();
        if active_count == 0 {
            return Err(WithdrawalAssignmentError::NoEligibleOperators {
                deposit_idx: deposit_entry.idx(),
            });
        }

        // Select a random operator from eligible ones
        let random_index = (rng.next_u32() as usize) % active_count;
        let current_assignee = eligible_operators
            .active_indices()
            .nth(random_index)
            .expect("random_index is within bounds of active_count");

        Ok(Self {
            deposit_entry: deposit_entry.clone(),
            withdrawal_cmd,
            current_assignee,
            previous_assignees,
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
    /// - `Err(WithdrawalAssignmentError)` - If the bitmap operation fails or no eligible operators
    ///   are available
    pub fn reassign(
        &mut self,
        new_operator_fee: BitcoinAmount,
        new_deadline: BitcoinBlockHeight,
        seed: L1BlockId,
        current_active_operators: &OperatorBitmap,
    ) -> Result<(), WithdrawalAssignmentError> {
        self.previous_assignees
            .try_set(self.current_assignee, true)
            .map_err(WithdrawalAssignmentError::BitmapError)?;

        // Use ChaChaRng with L1 block ID as seed for deterministic random selection
        let seed_bytes: [u8; 32] = Buf32::from(seed).into();
        let mut rng = ChaChaRng::from_seed(seed_bytes);

        // Use the already cached bitmap from DepositEntry instead of converting from Vec
        let mut eligible_operators = filter_eligible_operators(
            self.deposit_entry.notary_operators(),
            &self.previous_assignees,
            current_active_operators,
        )?;

        if eligible_operators.active_count() == 0 {
            // If no eligible operators left, clear previous assignees
            self.previous_assignees =
                OperatorBitmap::new_with_size(self.deposit_entry.notary_operators().len(), false);
            eligible_operators = filter_eligible_operators(
                self.deposit_entry.notary_operators(),
                &self.previous_assignees,
                current_active_operators,
            )?;
        }

        // If still no eligible operators, return error
        let active_count = eligible_operators.active_count();
        if active_count == 0 {
            return Err(WithdrawalAssignmentError::NoEligibleOperators {
                deposit_idx: self.deposit_entry.idx(),
            });
        }

        // Select a random operator from eligible ones
        let random_index = (rng.next_u32() as usize) % active_count;
        let new_assignee = eligible_operators
            .active_indices()
            .nth(random_index)
            .expect("random_index is within bounds of active_count");

        self.current_assignee = new_assignee;
        self.withdrawal_cmd.update_fee(new_operator_fee);
        self.exec_deadline = new_deadline;
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
/// - Removing completed assignments
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
    /// let active_operators = OperatorBitmap::new_with_size(3, true);
    /// let seed = L1BlockId::from([0u8; 32]);
    ///
    /// table.reassign_expired_assignments(current_height, &active_operators, seed)?;
    /// ```
    pub fn reassign_expired_assignments(
        &mut self,
        operator_fee: BitcoinAmount,
        current_height: BitcoinBlockHeight,
        deadline_duration: BitcoinBlockHeight,
        current_active_operators: &OperatorBitmap,
        seed: L1BlockId,
    ) -> Result<Vec<u32>, WithdrawalCommandError> {
        let mut reassigned_withdrawals = Vec::new();

        let new_deadline = current_height + deadline_duration;

        // Using iter_mut since we're only modifying non-sorting fields
        for assignment in self
            .assignments
            .iter_mut()
            .filter(|e| e.exec_deadline <= current_height)
        {
            assignment.reassign(operator_fee, new_deadline, seed, current_active_operators)?;
            reassigned_withdrawals.push(assignment.deposit_idx());
        }

        Ok(reassigned_withdrawals)
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::l1::{BitcoinBlockHeight, L1BlockId};
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
        let current_active_operators = deposit_entry.notary_operators().clone();

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
        assert_eq!(assignment.previous_assignees.active_count(), 0);
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

        let err = AssignmentEntry::create_with_random_assignment(
            deposit_entry.clone(),
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            WithdrawalAssignmentError::InsufficientActiveBitmapLength { .. }
        ));
    }

    #[test]
    fn test_reassign_success() {
        let mut arb = ArbitraryGenerator::new();

        // Keep generating deposit entries until we have at least 2 active operators
        let deposit_entry: DepositEntry = loop {
            let candidate: DepositEntry = arb.generate();
            if candidate.notary_operators().active_count() >= 2 {
                break candidate;
            }
        };

        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let exec_deadline: BitcoinBlockHeight = 100;
        let seed1: L1BlockId = arb.generate();
        let seed2: L1BlockId = arb.generate();

        // Use the deposit's notary operators as active operators
        let current_active_operators = deposit_entry.notary_operators().clone();
        let new_fee = BitcoinAmount::from_sat(20_000);

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        let original_assignee = assignment.current_assignee();
        assert_eq!(assignment.previous_assignees.active_count(), 0);

        // Reassign to a new operator
        let new_deadline: BitcoinBlockHeight = 200;
        let result = assignment.reassign(new_fee, new_deadline, seed2, &current_active_operators);
        assert!(result.is_ok());

        // Verify reassignment
        assert_eq!(assignment.previous_assignees.active_count(), 1);
        assert!(assignment.previous_assignees.is_active(original_assignee));
        assert_ne!(assignment.current_assignee(), original_assignee);
    }

    #[test]
    fn test_reassign_all_operators_exhausted() {
        let mut arb = ArbitraryGenerator::new();
        let mut deposit_entry: DepositEntry = arb.generate();

        // Force single operator for this test
        let operators = OperatorBitmap::new_with_size(1, true);
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

        let current_active_operators = OperatorBitmap::new_with_size(1, true); // Single operator with index 0

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            exec_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        // First reassignment should work (clears previous assignees and reassigns to same operator)
        let new_deadline: BitcoinBlockHeight = 200;
        let result = assignment.reassign(new_operator_fee, new_deadline, seed2, &current_active_operators);
        assert!(result.is_ok());

        // Should have cleared previous assignees and reassigned to the same operator
        assert_eq!(assignment.previous_assignees.active_count(), 0);
        assert_eq!(assignment.current_assignee(), 0); // Should be operator index 0
    }

    #[test]
    fn test_reassign_updates_deadline() {
        let mut arb = ArbitraryGenerator::new();

        // Keep generating deposit entries until we have at least 2 active operators
        let deposit_entry: DepositEntry = loop {
            let candidate: DepositEntry = arb.generate();
            if candidate.notary_operators().active_count() >= 2 {
                break candidate;
            }
        };

        let withdrawal_cmd: WithdrawalCommand = arb.generate();
        let initial_deadline: BitcoinBlockHeight = 100;
        let seed1: L1BlockId = arb.generate();
        let seed2: L1BlockId = arb.generate();

        // Use the deposit's notary operators as active operators
        let current_active_operators = deposit_entry.notary_operators().clone();
        let new_fee = BitcoinAmount::from_sat(20_000);

        let mut assignment = AssignmentEntry::create_with_random_assignment(
            deposit_entry,
            withdrawal_cmd,
            initial_deadline,
            &current_active_operators,
            seed1,
        )
        .unwrap();

        assert_eq!(assignment.exec_deadline(), initial_deadline);

        // Reassign with a new deadline
        let new_deadline: BitcoinBlockHeight = 250;
        let result = assignment.reassign(new_fee, new_deadline, seed2, &current_active_operators);
        assert!(result.is_ok());

        // Verify the deadline was updated
        assert_eq!(
            assignment.exec_deadline(),
            new_deadline,
            "Exec deadline should be updated to the new deadline after reassignment"
        );
        assert_ne!(
            assignment.exec_deadline(),
            initial_deadline,
            "Exec deadline should have changed from initial value"
        );
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
        let current_active_operators = deposit_entry.notary_operators().clone();

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
        let current_active_operators1 = deposit_entry1.notary_operators().clone();

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
        let current_active_operators2 = deposit_entry2.notary_operators().clone();

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
        let deadline_duration: BitcoinBlockHeight = 144;
        let result = table.reassign_expired_assignments(
            new_operator_fee,
            current_height,
            deadline_duration,
            &current_active_operators1,
            seed,
        );

        assert!(result.is_ok(), "Reassignment should succeed");

        // Check that expired assignment was reassigned
        let expired_assignment_after = table.get_assignment(expired_deposit_idx).unwrap();
        assert_eq!(
            expired_assignment_after.previous_assignees.active_count(),
            1
        );
        assert!(
            expired_assignment_after
                .previous_assignees
                .is_active(original_assignee)
        );
        // Verify the deadline was increased
        let expected_new_deadline = current_height + deadline_duration;
        assert_eq!(
            expired_assignment_after.exec_deadline(),
            expected_new_deadline,
            "Exec deadline should be increased after reassignment"
        );

        // Check that non-expired assignment was not reassigned
        let future_assignment_after = table.get_assignment(future_deposit_idx).unwrap();
        assert_eq!(future_assignment_after.previous_assignees.active_count(), 0);
        assert_eq!(
            future_assignment_after.current_assignee(),
            future_original_assignee
        );
    }
}
