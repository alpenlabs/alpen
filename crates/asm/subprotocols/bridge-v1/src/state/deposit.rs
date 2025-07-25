//! Bitcoin Deposit Management
//!
//! This module contains types and tables for managing Bitcoin deposits in the bridge.
//! Deposits represent Bitcoin UTXOs locked to N/N multisig addresses where N are the
//! notary operators. We preserve the historical operator set that controlled each deposit
//! since the operator set may change over time.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, OutputRef},
};

/// Bitcoin deposit entry containing UTXO reference and historical multisig operators.
///
/// Each deposit represents a Bitcoin UTXO that has been locked to an N/N multisig
/// address where N are the notary operators. The deposit tracks:
///
/// - **`deposit_idx`** - Unique identifier for this deposit
/// - **`output`** - Bitcoin UTXO reference (transaction hash + output index)
/// - **`notary_operators`** - The N operators that make up the N/N multisig
/// - **`amt`** - Amount of Bitcoin locked in this deposit
///
/// # Multisig Design
///
/// The `notary_operators` field preserves the historical set of operators that
/// formed the N/N multisig when this deposit was locked. Any one honest operator
/// from this set can properly process user withdrawals. We store this historical
/// set because the active operator set may change over time.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DepositEntry {
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

impl DepositEntry {
    /// Creates a new deposit entry with the specified parameters.
    ///
    /// # Parameters
    ///
    /// - `idx` - Unique deposit identifier
    /// - `output` - Bitcoin UTXO reference
    /// - `operators` - Historical set of operators that form the N/N multisig
    /// - `amt` - Amount of Bitcoin locked in the deposit
    ///
    /// # Returns
    ///
    /// A new [`DepositEntry`] instance.
    pub fn new(
        idx: u32,
        output: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> Self {
        Self {
            deposit_idx: idx,
            output,
            notary_operators: operators,
            amt,
        }
    }

    /// Returns the unique deposit identifier.
    ///
    /// # Returns
    ///
    /// The deposit index as [`u32`].
    pub fn idx(&self) -> u32 {
        self.deposit_idx
    }

    /// Returns a reference to the Bitcoin UTXO being tracked.
    ///
    /// # Returns
    ///
    /// Reference to the [`OutputRef`] containing transaction hash and output index.
    pub fn output(&self) -> &OutputRef {
        &self.output
    }

    /// Returns the historical set of operators that formed the N/N multisig.
    ///
    /// This preserves the specific operators who controlled the multisig when the
    /// deposit was locked. Any one honest operator from this set can properly
    /// process user withdrawals.
    ///
    /// # Returns
    ///
    /// Slice of [`OperatorIdx`] values representing the multisig operators.
    pub fn notary_operators(&self) -> &[OperatorIdx] {
        &self.notary_operators
    }

    /// Returns the amount of Bitcoin locked in this deposit.
    ///
    /// # Returns
    ///
    /// The deposit amount as [`BitcoinAmount`] (in satoshis).
    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

/// Table for managing Bitcoin deposits with efficient lookup operations.
///
/// This table maintains all deposits tracked by the bridge, providing efficient
/// insertion and lookup operations. The table automatically assigns unique indices
/// and maintains sorted order for binary search efficiency.
///
/// # Ordering Invariant
///
/// The deposits vector **MUST** remain sorted by deposit index at all times.
/// This invariant enables O(log n) lookup operations via binary search.
///
/// # Index Management
///
/// - `next_idx` tracks the next available deposit index for new deposits
/// - Indices can be assigned sequentially or at specific positions
/// - Out-of-order insertions are supported and maintain sorted order
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositsTable {
    /// Next unassigned deposit index for new registrations.
    next_idx: u32,

    /// Vector of deposit entries, sorted by deposit index.
    ///
    /// **Invariant**: MUST be sorted by `DepositEntry::deposit_idx` field.
    deposits: Vec<DepositEntry>,
}

impl DepositsTable {
    /// Creates a new empty deposits table.
    ///
    /// Initializes the table with no deposits and `next_idx` set to 0,
    /// ready for deposit registrations.
    ///
    /// # Returns
    ///
    /// A new empty [`DepositsTable`].
    pub fn new_empty() -> Self {
        Self {
            next_idx: 0,
            deposits: Vec::new(),
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

    /// Finds the position where a deposit with the given index exists or should be inserted.
    ///
    /// Uses binary search to efficiently locate the position.
    ///
    /// # Parameters
    ///
    /// - `idx` - The deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Ok(position)` if a deposit with this index exists
    /// - `Err(position)` where the deposit should be inserted to maintain sort order
    pub fn get_deposit_entry_pos(&self, idx: u32) -> Result<u32, u32> {
        self.deposits
            .binary_search_by_key(&idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
    }

    /// Retrieves a deposit entry by its unique index.
    ///
    /// Uses binary search for O(log n) lookup performance.
    ///
    /// # Parameters
    ///
    /// - `idx` - The unique deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&DepositEntry)` if the deposit exists
    /// - `None` if no deposit with the given index is found
    pub fn get_deposit(&self, idx: u32) -> Option<&DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &self.deposits[i as usize])
    }

    /// Retrieves a mutable reference to a deposit entry by its unique index.
    ///
    /// Uses binary search for O(log n) lookup performance.
    ///
    /// # Parameters
    ///
    /// - `idx` - The unique deposit index to search for
    ///
    /// # Returns
    ///
    /// - `Some(&mut DepositEntry)` if the deposit exists
    /// - `None` if no deposit with the given index is found
    pub fn get_deposit_mut(&mut self, idx: u32) -> Option<&mut DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &mut self.deposits[i as usize])
    }

    /// Returns an iterator over all deposit indices.
    ///
    /// The indices are returned in sorted order due to the table's invariant.
    ///
    /// # Returns
    ///
    /// Iterator yielding each deposit's unique index.
    pub fn deposit_indices(&self) -> impl Iterator<Item = u32> + '_ {
        self.deposits.iter().map(|e| e.deposit_idx)
    }

    /// Retrieves a deposit entry by its position in the internal vector.
    ///
    /// This method accesses deposits by their storage position rather than their
    /// logical index. Useful for iteration or when the position is known.
    ///
    /// # Parameters
    ///
    /// - `pos` - The position in the internal vector (0-based)
    ///
    /// # Returns
    ///
    /// - `Some(&DepositEntry)` if the position is valid
    /// - `None` if the position is out of bounds
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&DepositEntry> {
        self.deposits.get(pos as usize)
    }

    /// Creates a new deposit with the next available index.
    ///
    /// This is the most efficient way to add deposits when order doesn't matter,
    /// as no binary search or insertion is required.
    ///
    /// # Parameters
    ///
    /// - `tx_ref` - Bitcoin UTXO reference for the deposit
    /// - `operators` - Historical set of operators that form the N/N multisig
    /// - `amt` - Amount of Bitcoin locked in the deposit
    ///
    /// # Returns
    ///
    /// The unique index assigned to the new deposit.
    pub fn create_next_deposit(
        &mut self,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> u32 {
        let idx = self.next_idx();
        let deposit_entry = DepositEntry::new(idx, tx_ref, operators, amt);
        self.deposits.push(deposit_entry);
        self.next_idx += 1;
        idx
    }

    /// Attempts to create a deposit at a specific index.
    ///
    /// This method allows inserting deposits at specific indices, which is useful
    /// for maintaining consistency when deposits are processed out of order.
    /// If the requested index is beyond `next_idx`, it updates `next_idx` accordingly.
    ///
    /// # Parameters
    ///
    /// - `idx` - The specific index to assign to the deposit
    /// - `tx_ref` - Bitcoin UTXO reference for the deposit
    /// - `operators` - Historical set of operators that form the N/N multisig
    /// - `amt` - Amount of Bitcoin locked in the deposit
    ///
    /// # Returns
    ///
    /// - `true` if the deposit was successfully created
    /// - `false` if a deposit with this index already exists
    pub fn try_create_deposit_at(
        &mut self,
        idx: u32,
        tx_ref: OutputRef,
        operators: Vec<OperatorIdx>,
        amt: BitcoinAmount,
    ) -> bool {
        // Happy case, if we're creating the next entry we can skip the binary
        // search.  This should be most cases, where there isn't concurrent
        // interleaved deposit processing.
        if idx == self.next_idx {
            self.create_next_deposit(tx_ref, operators, amt);
            return true;
        }

        // Slow path.
        match self.get_deposit_entry_pos(idx) {
            Ok(_) => false,
            Err(pos) => {
                let entry = DepositEntry::new(idx, tx_ref, operators, amt);
                self.deposits.insert(pos as usize, entry);

                // Tricky bookkeeping.
                if idx >= self.next_idx {
                    self.next_idx = u32::max(self.next_idx, idx) + 1;
                }

                true
            }
        }
    }

    /// Returns the next available deposit index.
    ///
    /// This is the index that will be assigned to the next deposit
    /// created via [`create_next_deposit`].
    ///
    /// # Returns
    ///
    /// The next available deposit index as [`u32`].
    pub fn next_idx(&self) -> u32 {
        self.next_idx
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

    /// Removes a deposit entry from the table by its index.
    ///
    /// This method locates and removes the deposit with the specified index
    /// from the table. Uses binary search for efficient lookup.
    ///
    /// # Parameters
    ///
    /// - `idx` - The unique deposit index to remove
    ///
    /// # Returns
    ///
    /// - `Some(DepositEntry)` if the deposit was found and removed
    /// - `None` if no deposit with the given index exists
    pub fn remove_deposit(&mut self, idx: u32) -> Option<DepositEntry> {
        match self.get_deposit_entry_pos(idx) {
            Ok(pos) => Some(self.deposits.remove(pos as usize)),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::{
        buf::Buf32,
        l1::{BitcoinAmount, OutputRef},
    };

    use super::*;

    fn create_test_output_ref(idx: u32) -> OutputRef {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[0] = idx as u8;
        OutputRef::new(Buf32::from(hash_bytes).into(), idx)
    }

    #[test]
    fn test_deposit_entry_new() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![0, 1, 2];
        let amount = BitcoinAmount::from_sat(1000);

        let deposit = DepositEntry::new(5, output_ref, operators.clone(), amount);

        assert_eq!(deposit.idx(), 5);
        assert_eq!(deposit.output(), &output_ref);
        assert_eq!(deposit.notary_operators(), &operators);
        assert_eq!(deposit.amt(), amount);
    }

    #[test]
    fn test_deposit_entry_getters() {
        let output_ref = create_test_output_ref(42);
        let operators = vec![5, 10, 15];
        let amount = BitcoinAmount::from_sat(50000);

        let deposit = DepositEntry::new(100, output_ref, operators.clone(), amount);

        // Test all getter methods
        assert_eq!(deposit.idx(), 100);
        assert_eq!(deposit.output(), &output_ref);
        assert_eq!(deposit.notary_operators(), &operators);
        assert_eq!(deposit.amt(), amount);
    }

    #[test]
    fn test_deposit_entry_with_empty_operators() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![];
        let amount = BitcoinAmount::from_sat(100);

        let deposit = DepositEntry::new(0, output_ref, operators.clone(), amount);

        assert_eq!(deposit.notary_operators(), &operators);
        assert!(deposit.notary_operators().is_empty());
    }

    #[test]
    fn test_deposit_entry_with_single_operator() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![7];
        let amount = BitcoinAmount::from_sat(2500);

        let deposit = DepositEntry::new(25, output_ref, operators.clone(), amount);

        assert_eq!(deposit.notary_operators(), &operators);
        assert_eq!(deposit.notary_operators().len(), 1);
        assert_eq!(deposit.notary_operators()[0], 7);
    }

    #[test]
    fn test_deposit_entry_with_zero_amount() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(0);

        let deposit = DepositEntry::new(1, output_ref, operators, amount);

        assert_eq!(deposit.amt(), BitcoinAmount::from_sat(0));
    }

    #[test]
    fn test_deposit_entry_with_large_amount() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(2_100_000_000_000_000); // 21M BTC in sats

        let deposit = DepositEntry::new(1, output_ref, operators, amount);

        assert_eq!(deposit.amt(), amount);
    }

    #[test]
    fn test_deposit_entry_clone() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![1, 2, 3];
        let amount = BitcoinAmount::from_sat(1000);

        let deposit1 = DepositEntry::new(10, output_ref, operators.clone(), amount);
        let deposit2 = deposit1.clone();

        assert_eq!(deposit1.idx(), deposit2.idx());
        assert_eq!(deposit1.output(), deposit2.output());
        assert_eq!(deposit1.notary_operators(), deposit2.notary_operators());
        assert_eq!(deposit1.amt(), deposit2.amt());
    }

    #[test]
    fn test_deposit_entry_equality() {
        let output_ref = create_test_output_ref(1);
        let operators = vec![1, 2];
        let amount = BitcoinAmount::from_sat(500);

        let deposit1 = DepositEntry::new(5, output_ref, operators.clone(), amount);
        let deposit2 = DepositEntry::new(5, output_ref, operators, amount);

        assert_eq!(deposit1, deposit2);
    }

    #[test]
    fn test_deposit_entry_inequality() {
        let output_ref1 = create_test_output_ref(1);
        let output_ref2 = create_test_output_ref(2);
        let operators = vec![1];
        let amount = BitcoinAmount::from_sat(1000);

        let deposit1 = DepositEntry::new(1, output_ref1, operators.clone(), amount);
        let deposit2 = DepositEntry::new(2, output_ref2, operators, amount);

        assert_ne!(deposit1, deposit2);
    }

    #[test]
    fn test_deposits_table_new_empty() {
        let table = DepositsTable::new_empty();

        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
        assert_eq!(table.next_idx(), 0);
        assert_eq!(table.deposits().count(), 0);
    }

    #[test]
    fn test_deposits_table_create_next_deposit() {
        let mut table = DepositsTable::new_empty();

        let output_ref1 = create_test_output_ref(1);
        let operators1 = vec![0, 1];
        let amount1 = BitcoinAmount::from_sat(1000);

        let idx1 = table.create_next_deposit(output_ref1, operators1.clone(), amount1);

        assert_eq!(idx1, 0);
        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());
        assert_eq!(table.next_idx(), 1);

        let deposit = table.get_deposit(0).unwrap();
        assert_eq!(deposit.idx(), 0);
        assert_eq!(deposit.output(), &output_ref1);
        assert_eq!(deposit.notary_operators(), &operators1);
        assert_eq!(deposit.amt(), amount1);

        // Add second deposit
        let output_ref2 = create_test_output_ref(2);
        let operators2 = vec![2, 3];
        let amount2 = BitcoinAmount::from_sat(2000);

        let idx2 = table.create_next_deposit(output_ref2, operators2.clone(), amount2);

        assert_eq!(idx2, 1);
        assert_eq!(table.len(), 2);
        assert_eq!(table.next_idx(), 2);

        let deposit2 = table.get_deposit(1).unwrap();
        assert_eq!(deposit2.idx(), 1);
        assert_eq!(deposit2.notary_operators(), &operators2);
        assert_eq!(deposit2.amt(), amount2);
    }

    #[test]
    fn test_deposits_table_try_create_deposit_at_sequential() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create deposit at index 0 (next_idx)
        let success = table.try_create_deposit_at(0, output_ref, operators.clone(), amount);
        assert!(success);
        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx(), 1);

        // Create deposit at index 1 (next_idx)
        let success = table.try_create_deposit_at(1, output_ref, operators, amount);
        assert!(success);
        assert_eq!(table.len(), 2);
        assert_eq!(table.next_idx(), 2);
    }

    #[test]
    fn test_deposits_table_try_create_deposit_at_duplicate() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create deposit at index 0
        let success = table.try_create_deposit_at(0, output_ref, operators.clone(), amount);
        assert!(success);
        assert_eq!(table.len(), 1);

        // Try to create another deposit at the same index (should fail)
        let success = table.try_create_deposit_at(0, output_ref, operators, amount);
        assert!(!success);
        assert_eq!(table.len(), 1); // Should remain unchanged
    }

    #[test]
    fn test_deposits_table_try_create_deposit_at_future_index() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create deposit at a future index
        let success = table.try_create_deposit_at(5, output_ref, operators, amount);
        assert!(success);
        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx(), 6); // Should update to max(next_idx, idx) + 1

        let deposit = table.get_deposit(5).unwrap();
        assert_eq!(deposit.idx(), 5);
    }

    #[test]
    fn test_deposits_table_try_create_deposit_at_out_of_order() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create deposits out of order
        assert!(table.try_create_deposit_at(5, output_ref, operators.clone(), amount));
        assert!(table.try_create_deposit_at(2, output_ref, operators.clone(), amount));
        assert!(table.try_create_deposit_at(8, output_ref, operators.clone(), amount));
        assert!(table.try_create_deposit_at(1, output_ref, operators, amount));

        assert_eq!(table.len(), 4);
        assert_eq!(table.next_idx(), 9); // Should be max(idx) + 1

        // Verify they are sorted
        let deposits: Vec<_> = table.deposits().collect();
        assert_eq!(deposits[0].idx(), 1);
        assert_eq!(deposits[1].idx(), 2);
        assert_eq!(deposits[2].idx(), 5);
        assert_eq!(deposits[3].idx(), 8);
    }

    #[test]
    fn test_deposits_table_get_deposit() {
        let mut table = DepositsTable::new_empty();

        let output_ref1 = create_test_output_ref(1);
        let output_ref2 = create_test_output_ref(2);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        table.create_next_deposit(output_ref1, operators.clone(), amount);
        table.create_next_deposit(output_ref2, operators, amount);

        // Test existing deposits
        let deposit0 = table.get_deposit(0).unwrap();
        assert_eq!(deposit0.idx(), 0);

        let deposit1 = table.get_deposit(1).unwrap();
        assert_eq!(deposit1.idx(), 1);

        // Test non-existing deposits
        assert!(table.get_deposit(2).is_none());
        assert!(table.get_deposit(100).is_none());
    }

    #[test]
    fn test_deposits_table_get_deposit_mut() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        table.create_next_deposit(output_ref, operators, amount);

        // Test mutable access exists
        let deposit = table.get_deposit_mut(0);
        assert!(deposit.is_some());

        // Test non-existing deposit
        assert!(table.get_deposit_mut(100).is_none());
    }

    #[test]
    fn test_deposits_table_get_entry_at_pos() {
        let mut table = DepositsTable::new_empty();

        let output_ref1 = create_test_output_ref(1);
        let output_ref2 = create_test_output_ref(2);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        table.create_next_deposit(output_ref1, operators.clone(), amount);
        table.create_next_deposit(output_ref2, operators, amount);

        // Test valid positions
        let deposit0 = table.get_entry_at_pos(0).unwrap();
        assert_eq!(deposit0.idx(), 0);

        let deposit1 = table.get_entry_at_pos(1).unwrap();
        assert_eq!(deposit1.idx(), 1);

        // Test invalid positions
        assert!(table.get_entry_at_pos(2).is_none());
        assert!(table.get_entry_at_pos(100).is_none());
    }

    #[test]
    fn test_deposits_table_deposit_indices() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create deposits with specific indices out of order
        table.try_create_deposit_at(0, output_ref, operators.clone(), amount);
        table.try_create_deposit_at(5, output_ref, operators.clone(), amount);
        table.try_create_deposit_at(2, output_ref, operators, amount);

        let indices: Vec<_> = table.deposit_indices().collect();
        assert_eq!(indices, vec![0, 2, 5]); // Should be sorted by deposit_idx
    }

    #[test]
    fn test_deposits_table_deposits_iterator() {
        let mut table = DepositsTable::new_empty();

        let output_ref1 = create_test_output_ref(1);
        let output_ref2 = create_test_output_ref(2);
        let output_ref3 = create_test_output_ref(3);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        table.create_next_deposit(output_ref1, operators.clone(), amount);
        table.create_next_deposit(output_ref2, operators.clone(), amount);
        table.create_next_deposit(output_ref3, operators, amount);

        let deposits: Vec<_> = table.deposits().collect();
        assert_eq!(deposits.len(), 3);
        assert_eq!(deposits[0].idx(), 0);
        assert_eq!(deposits[1].idx(), 1);
        assert_eq!(deposits[2].idx(), 2);

        // Test iterator is lazy - calling it multiple times should work
        let deposits2: Vec<_> = table.deposits().collect();
        assert_eq!(deposits.len(), deposits2.len());
    }

    #[test]
    fn test_deposits_table_get_deposit_entry_pos() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        table.try_create_deposit_at(0, output_ref, operators.clone(), amount);
        table.try_create_deposit_at(5, output_ref, operators, amount);

        // Test existing indices return Ok with position
        assert_eq!(table.get_deposit_entry_pos(0), Ok(0));
        assert_eq!(table.get_deposit_entry_pos(5), Ok(1));

        // Test non-existing indices return Err with insertion position
        assert_eq!(table.get_deposit_entry_pos(3), Err(1)); // Would insert at position 1
        assert_eq!(table.get_deposit_entry_pos(10), Err(2)); // Would insert at position 2
        assert_eq!(table.get_deposit_entry_pos(0), Ok(0)); // Edge case: first element
    }

    #[test]
    fn test_deposits_table_binary_search_efficiency() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create many deposits to test binary search
        for i in (0..100).step_by(2) {
            table.try_create_deposit_at(i, output_ref, operators.clone(), amount);
        }

        assert_eq!(table.len(), 50);

        // Test lookups work correctly
        assert!(table.get_deposit(0).is_some());
        assert!(table.get_deposit(50).is_some());
        assert!(table.get_deposit(98).is_some());

        // Test odd numbers don't exist
        assert!(table.get_deposit(1).is_none());
        assert!(table.get_deposit(51).is_none());
        assert!(table.get_deposit(99).is_none());
    }

    #[test]
    fn test_deposits_table_large_indices() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Test with large indices
        let large_idx = u32::MAX - 1;
        let success = table.try_create_deposit_at(large_idx, output_ref, operators, amount);
        assert!(success);

        assert_eq!(table.len(), 1);
        assert_eq!(table.next_idx(), u32::MAX);

        let deposit = table.get_deposit(large_idx).unwrap();
        assert_eq!(deposit.idx(), large_idx);
    }

    #[test]
    fn test_deposits_table_many_operators() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = (0..100).collect::<Vec<_>>(); // 100 operators
        let amount = BitcoinAmount::from_sat(1000);

        let idx = table.create_next_deposit(output_ref, operators.clone(), amount);

        let deposit = table.get_deposit(idx).unwrap();
        assert_eq!(deposit.notary_operators().len(), 100);
        assert_eq!(deposit.notary_operators(), &operators);
    }

    #[test]
    fn test_deposits_table_edge_cases() {
        let mut table = DepositsTable::new_empty();

        let output_ref = create_test_output_ref(1);
        let operators = vec![0];

        // Test minimum amount
        let min_amount = BitcoinAmount::from_sat(1);
        table.create_next_deposit(output_ref, operators.clone(), min_amount);

        let deposit = table.get_deposit(0).unwrap();
        assert_eq!(deposit.amt(), min_amount);

        // Test with no operators
        let empty_operators = vec![];
        table.create_next_deposit(output_ref, empty_operators, min_amount);

        let deposit = table.get_deposit(1).unwrap();
        assert!(deposit.notary_operators().is_empty());
    }

    #[test]
    fn test_deposits_table_remove_deposit() {
        let mut table = DepositsTable::new_empty();

        let output_ref1 = create_test_output_ref(1);
        let output_ref2 = create_test_output_ref(2);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);

        // Create two deposits
        table.create_next_deposit(output_ref1, operators.clone(), amount);
        table.create_next_deposit(output_ref2, operators, amount);

        assert_eq!(table.len(), 2);

        // Remove the first deposit
        let removed_deposit = table.remove_deposit(0).unwrap();
        assert_eq!(removed_deposit.idx(), 0);
        assert_eq!(table.len(), 1);

        // Verify the deposit is no longer accessible
        assert!(table.get_deposit(0).is_none());
        
        // Verify the second deposit is still there
        assert!(table.get_deposit(1).is_some());

        // Try to remove a non-existent deposit
        assert!(table.remove_deposit(10).is_none());
        assert_eq!(table.len(), 1);
    }
}
