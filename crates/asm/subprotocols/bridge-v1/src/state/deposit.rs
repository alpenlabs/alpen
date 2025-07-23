//! Deposit-related types and tables.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    bridge::OperatorIdx,
    l1::{BitcoinAmount, OutputRef},
};

/// Container for the state machine of a deposit factory.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct DepositEntry {
    deposit_idx: u32,

    /// The outpoint that this deposit entry references.
    output: OutputRef,

    /// List of notary operators, by their indexes.
    // TODO convert this to a windowed bitmap or something
    notary_operators: Vec<OperatorIdx>,

    /// Deposit amount, in the native asset.
    amt: BitcoinAmount,
}

impl DepositEntry {
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

    pub fn idx(&self) -> u32 {
        self.deposit_idx
    }

    pub fn output(&self) -> &OutputRef {
        &self.output
    }

    pub fn notary_operators(&self) -> &[OperatorIdx] {
        &self.notary_operators
    }

    pub fn amt(&self) -> BitcoinAmount {
        self.amt
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct DepositsTable {
    /// Next unassigned deposit index.
    next_idx: u32,

    /// Deposit table.
    ///
    /// MUST be sorted by `deposit_idx`.
    deposits: Vec<DepositEntry>,
}

impl DepositsTable {
    pub fn new_empty() -> Self {
        Self {
            next_idx: 0,
            deposits: Vec::new(),
        }
    }

    /// Returns the number of deposit entries being tracked.
    pub fn len(&self) -> u32 {
        self.deposits.len() as u32
    }

    /// Returns if the deposit table is empty.  This is practically probably
    /// never going to be true.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gets the position in the deposit table of a hypothetical deposit entry
    /// index.
    pub fn get_deposit_entry_pos(&self, idx: u32) -> Result<u32, u32> {
        self.deposits
            .binary_search_by_key(&idx, |e| e.deposit_idx)
            .map(|i| i as u32)
            .map_err(|i| i as u32)
    }

    /// Gets a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit(&self, idx: u32) -> Option<&DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &self.deposits[i as usize])
    }

    /// Gets a mut ref to a deposit from the table by its idx.
    ///
    /// Does a binary search.
    pub fn get_deposit_mut(&mut self, idx: u32) -> Option<&mut DepositEntry> {
        self.get_deposit_entry_pos(idx)
            .ok()
            .map(|i| &mut self.deposits[i as usize])
    }

    pub fn get_all_deposits_idxs_iters_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.deposits.iter().map(|e| e.deposit_idx)
    }

    /// Gets a deposit entry by its internal position, *ignoring* the indexes.
    pub fn get_entry_at_pos(&self, pos: u32) -> Option<&DepositEntry> {
        self.deposits.get(pos as usize)
    }

    /// Adds a new deposit to the table and returns the index of the new deposit.
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

    /// Tries to create a deposit entry at a specific idx.  If the entry requested if after the
    /// `next_entry`, then updates it to be equal to that.
    ///
    /// Returns if we inserted it successfully.
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

    pub fn next_idx(&self) -> u32 {
        self.next_idx
    }

    pub fn deposits(&self) -> impl Iterator<Item = &DepositEntry> {
        self.deposits.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_primitives::{
        buf::Buf32,
        l1::{BitcoinAmount, OutputRef},
    };

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
    fn test_deposits_table_get_all_deposits_idxs_iters_iter() {
        let mut table = DepositsTable::new_empty();
        
        let output_ref = create_test_output_ref(1);
        let operators = vec![0];
        let amount = BitcoinAmount::from_sat(1000);
        
        // Create deposits with specific indices out of order
        table.try_create_deposit_at(0, output_ref, operators.clone(), amount);
        table.try_create_deposit_at(5, output_ref, operators.clone(), amount);
        table.try_create_deposit_at(2, output_ref, operators, amount);
        
        let indices: Vec<_> = table.get_all_deposits_idxs_iters_iter().collect();
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
}
