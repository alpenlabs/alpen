//! Bitcoin Deposit Management
//!
//! This module contains types and tables for managing Bitcoin deposits in the bridge.
//! Deposits represent Bitcoin UTXOs locked to N/N multisig addresses where N are the
//! notary operators. We preserve the historical operator set that controlled each deposit
//! since the operator set may change over time.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use crate::errors::DepositValidationError;
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, OutputRef},
    sorted_vec::SortedVec,
};

/// Bitcoin deposit entry containing UTXO reference and historical multisig operators.
///
/// Each deposit represents a Bitcoin UTXO that has been locked to an N/N multisig
/// address where N are the notary operators. The deposit tracks:
///
/// - **`deposit_idx`** - Unique identifier assigned by the bridge for this deposit
/// - **`output`** - Bitcoin UTXO reference (transaction hash + output index)
/// - **`notary_operators`** - The N operators that make up the N/N multisig
/// - **`amt`** - Amount of Bitcoin locked in this deposit
///
/// # Index Assignment
///
/// The `deposit_idx` is assigned by the bridge and provided in the deposit transaction.
/// The bridge determines the indexing strategy, which may be based on either
/// `DepositRequestTransaction` or `DepositTransaction` ordering, depending on the
/// bridge's implementation needs.
///
/// This bridge-controlled ordering is essential for the stake chain to maintain
/// consistent deposit sequencing across all participants.
///
/// # Multisig Design
///
/// The `notary_operators` field preserves the historical set of operators that
/// formed the N/N multisig when this deposit was locked. Any one honest operator
/// from this set can properly process user withdrawals. We store this historical
/// set because the active operator set may change over time.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DepositEntry {
    /// Unique deposit identifier assigned by the bridge and provided in the deposit transaction.
    deposit_idx: u32,

    /// Bitcoin UTXO reference (transaction hash + output index).
    output: OutputRef,

    /// Historical set of operators that formed the N/N multisig for this deposit.
    ///
    /// This preserves the specific operators who controlled the multisig when the
    /// deposit was locked, since the active operator set may change over time.
    /// Any one honest operator from this set can process user withdrawals.
    ///
    /// TODO: Convert this to a windowed bitmap for better memory efficiency.
    notary_operators: Vec<OperatorIdx>,

    /// Amount of Bitcoin locked in this deposit (in satoshis).
    amt: BitcoinAmount,
}

impl PartialOrd for DepositEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DepositEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.idx().cmp(&other.idx())
    }
}

impl DepositEntry {
    /// Creates a new deposit entry with the specified parameters.
    ///
    /// # Parameters
    ///
    /// - `idx` - Unique deposit identifier
    /// - `output` - Bitcoin UTXO reference
    /// - `operators` - Historical set of operators that form the N/N multisig (must be non-empty)
    /// - `amt` - Amount of Bitcoin locked in the deposit
    ///
    /// # Returns
    ///
    /// - `Ok(DepositEntry)` if the parameters are valid
    /// - `Err(DepositValidationError::EmptyOperators)` if the operators list is empty
    ///
    /// # Errors
    ///
    /// Returns [`DepositValidationError::EmptyOperators`] if the operators vector is empty.
    /// Each deposit must have at least one notary operator.
    pub fn new(
        idx: u32,
        output: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> Result<Self, DepositValidationError> {
        if operators.is_empty() {
            return Err(DepositValidationError::EmptyOperators);
        }

        Ok(Self {
            deposit_idx: idx,
            output,
            notary_operators: operators,
            amt,
        })
    }

    /// Returns the unique deposit identifier.
    pub fn idx(&self) -> u32 {
        self.deposit_idx
    }

    /// Returns a reference to the Bitcoin UTXO being tracked.
    pub fn output(&self) -> &OutputRef {
        &self.output
    }

    /// Returns the historical set of operators that formed the N/N multisig.
    pub fn notary_operators(&self) -> &[OperatorIdx] {
        &self.notary_operators
    }

    /// Returns the amount of Bitcoin locked in this deposit.
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

impl<'a> Arbitrary<'a> for DepositEntry {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a random deposit index
        let deposit_idx: u32 = u.arbitrary()?;

        // Generate a random Bitcoin UTXO reference
        let output: OutputRef = u.arbitrary()?;

        // Generate a random number of notary operators between 1 and 20
        let num_operators = u.int_in_range(1..=20)?;
        let mut notary_operators = Vec::with_capacity(num_operators);

        for _ in 0..num_operators {
            let operator_idx: OperatorIdx = u.arbitrary()?;
            notary_operators.push(operator_idx);
        }

        // Generate a random Bitcoin amount (between 1 satoshi and 21 million BTC)
        let amount: BitcoinAmount = u.arbitrary()?;

        // Create the DepositEntry - this should not fail since we ensure operators is non-empty
        Self::new(deposit_idx, output, notary_operators, amount)
            .map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

/// Table for managing Bitcoin deposits with efficient lookup operations.
///
/// This table maintains all deposits tracked by the bridge, providing efficient
/// insertion and lookup operations. The table maintains sorted order for binary search efficiency.
///
/// # Ordering Invariant
///
/// The deposits vector **MUST** remain sorted by deposit index at all times.
/// This invariant enables O(log n) lookup operations via binary search.
///
/// # Index Management
///
/// - Deposit indices are provided by the caller (from DepositInfo)
/// - Out-of-order insertions are supported and maintain sorted order
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositsTable {
    /// Vector of deposit entries, sorted by deposit index.
    ///
    /// **Invariant**: MUST be sorted by `DepositEntry::deposit_idx` field.
    deposits: SortedVec<DepositEntry>,
}

impl DepositsTable {
    /// Creates a new empty deposits table.
    ///
    /// Initializes the table with no deposits, ready for deposit registrations.
    ///
    /// # Returns
    ///
    /// A new empty [`DepositsTable`].
    pub fn new_empty() -> Self {
        Self {
            deposits: SortedVec::new_empty(),
        }
    }

    /// Returns the number of deposits being tracked.
    ///
    /// # Returns
    ///
    /// The total count of deposits in the table as [`u32`].
    pub fn len(&self) -> u32 {
        self.deposits.len() as u32
    }

    /// Returns whether the deposits table is empty.
    ///
    /// In practice, this will typically return `false` once deposits start
    /// being processed by the bridge.
    ///
    /// # Returns
    ///
    /// `true` if no deposits are tracked, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retrieves a deposit entry by its index using binary search.
    ///
    /// Performs an efficient O(log n) lookup to find the deposit with the specified index.
    /// Takes advantage of the sorted order invariant maintained by the deposits vector.
    ///
    /// # Parameters
    ///
    /// - `deposit_idx` - The unique deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&DepositEntry)` if a deposit with the given index exists
    /// - `None` if no deposit with the given index is found
    pub fn get_deposit(&self, deposit_idx: u32) -> Option<&DepositEntry> {
        self.deposits
            .as_slice()
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx)
            .ok()
            .map(|pos| &self.deposits.as_slice()[pos])
    }

    /// Returns an iterator over all deposit entries.
    ///
    /// The entries are returned in sorted order by deposit index.
    ///
    /// # Returns
    ///
    /// Iterator yielding references to all [`DepositEntry`] instances.
    pub fn deposits(&self) -> impl Iterator<Item = &DepositEntry> {
        self.deposits.iter()
    }

    /// Inserts a deposit entry into the table at the correct position.
    ///
    /// Takes an existing [`DepositEntry`] and inserts it into the deposits table,
    /// maintaining sorted order by deposit index. Uses binary search to find the
    /// optimal insertion point.
    ///
    /// # Parameters
    ///
    /// - `entry` - The deposit entry to insert
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the deposit was successfully inserted
    /// - `Err(DepositValidationError::DepositIdxAlreadyExists)` if a deposit with this index
    ///   already exists
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut table = DepositsTable::new_empty();
    /// let entry = DepositEntry::new(0, output_ref, operators, amount);
    /// let result = table.insert_deposit(entry);
    /// assert!(result.is_ok());
    /// ```
    pub fn insert_deposit(&mut self, entry: DepositEntry) -> Result<(), DepositValidationError> {
        let idx = entry.deposit_idx;
        match self.get_deposit(idx) {
            Some(_) => Err(DepositValidationError::DepositIdxAlreadyExists(idx)),
            None => {
                // SortedVec handles insertion and maintains sorted order
                self.deposits.insert(entry);
                Ok(())
            }
        }
    }

    /// Removes and returns the oldest deposit from the table.
    ///
    /// Since the table is sorted by deposit index, the oldest deposit (with the
    /// smallest deposit_idx) is always at position 0. This method removes and
    /// returns that deposit.
    ///
    /// # Returns
    ///
    /// - `Some(DepositEntry)` if there are deposits in the table
    /// - `None` if the table is empty
    pub fn remove_oldest_deposit(&mut self) -> Option<DepositEntry> {
        if self.deposits.is_empty() {
            None
        } else {
            // Get the first (oldest) deposit and remove it
            let oldest = self.deposits.as_slice()[0].clone();
            self.deposits.remove(&oldest);
            Some(oldest)
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::l1::BitcoinAmount;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_deposit_entry_new_empty_operators() {
        let output: OutputRef = ArbitraryGenerator::new().generate();
        let operators = vec![];
        let amount = BitcoinAmount::from_sat(1_000_000);

        let result = DepositEntry::new(1, output, operators, amount);
        assert!(matches!(
            result,
            Err(DepositValidationError::EmptyOperators)
        ));
    }

    #[test]
    fn test_deposits_table_insert_single() {
        let mut table = DepositsTable::new_empty();
        let entry: DepositEntry = ArbitraryGenerator::new().generate();

        let result = table.insert_deposit(entry.clone());
        assert!(result.is_ok());

        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());

        let retrieved = table
            .get_deposit(entry.idx())
            .expect("must find inserted deposit");
        assert_eq!(&entry, retrieved);
    }

    #[test]
    fn test_deposits_table_insert_duplicate_idx() {
        let mut table = DepositsTable::new_empty();

        let entry1: DepositEntry = ArbitraryGenerator::new().generate();
        let deposit_idx = entry1.deposit_idx;
        assert!(table.insert_deposit(entry1).is_ok());

        let mut entry2: DepositEntry = ArbitraryGenerator::new().generate();
        entry2.deposit_idx = deposit_idx; // Force duplicate index

        let result = table.insert_deposit(entry2.clone());
        assert!(matches!(
            result,
            Err(DepositValidationError::DepositIdxAlreadyExists(idx)) if idx == deposit_idx
        ));
    }

    #[test]
    fn test_deposits_table_inserts_and_removals() {
        let mut table = DepositsTable::new_empty();
        let mut arb = ArbitraryGenerator::new();

        let len = 10;
        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
        for _ in 0..len {
            let entry: DepositEntry = arb.generate();
            assert!(table.insert_deposit(entry).is_ok());
        }
        assert_eq!(table.len(), len);

        // Verify they are stored in sorted order
        let deposit_indices: Vec<_> = table.deposits().map(|e| e.deposit_idx).collect();
        assert!(deposit_indices.is_sorted());

        let mut removed_indices = Vec::new();
        for i in 0..len {
            let removed = table.remove_oldest_deposit();
            assert!(removed.is_some());
            let idx = removed.unwrap().idx();
            removed_indices.push(idx);
            assert!(table.len() == (len - i - 1));
        }
        assert!(table.remove_oldest_deposit().is_none());

        assert!(removed_indices.is_sorted());
    }
}
