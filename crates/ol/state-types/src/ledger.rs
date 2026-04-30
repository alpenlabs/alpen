//! Accounts system types.
//!
//! This uses the "transitional" types described in the OL STF spec.

use ssz_types::VariableList;
use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount};
use strata_ledger_types::{IAccountState, NewAccountData, StateError, StateResult};

use crate::ssz_generated::ssz::state::{OLAccountState, TsnlAccountEntry, TsnlLedgerAccountsTable};

impl TsnlLedgerAccountsTable {
    /// Creates a new empty table.
    ///
    /// This reserves serials for system accounts with 0 values.
    pub fn new_empty() -> Self {
        Self {
            accounts: VariableList::empty(),
        }
    }

    fn get_acct_entry_idx(&self, id: &AccountId) -> Option<usize> {
        self.accounts.binary_search_by_key(id, |e| e.id).ok()
    }

    fn get_acct_entry(&self, id: &AccountId) -> Option<&TsnlAccountEntry> {
        let idx = self.get_acct_entry_idx(id)?;
        self.accounts.get(idx)
    }

    fn get_acct_entry_mut(&mut self, id: &AccountId) -> Option<&mut TsnlAccountEntry> {
        let idx = self.get_acct_entry_idx(id)?;
        self.accounts.get_mut(idx)
    }

    pub fn get_account_state(&self, id: &AccountId) -> Option<&OLAccountState> {
        self.get_acct_entry(id).map(|e| &e.state)
    }

    pub fn get_account_state_mut(&mut self, id: &AccountId) -> Option<&mut OLAccountState> {
        self.get_acct_entry_mut(id).map(|e| &mut e.state)
    }

    /// Creates a new account.
    ///
    /// This does not check serial uniqueness/ordering.
    pub fn create_account(&mut self, id: AccountId, acct_state: OLAccountState) -> StateResult<()> {
        // Figure out where we're supposed to put it.
        let insert_idx = match self.accounts.binary_search_by_key(&id, |e| e.id) {
            Ok(_) => return Err(StateError::AccountExists(id)),
            Err(i) => i,
        };

        // Actually insert the entry.
        // VariableList doesn't have insert, but it has push.
        // Since we need to maintain sorted order, we collect to Vec, insert, and convert back.
        let entry = TsnlAccountEntry::new(id, acct_state);
        let mut accounts_vec: Vec<_> = self.accounts.iter().cloned().collect();
        accounts_vec.insert(insert_idx, entry);
        self.accounts = accounts_vec.try_into().expect("accounts should fit");

        // Sanity check.
        assert!(
            self.accounts.is_sorted_by_key(|e| e.id),
            "ol/state: accounts table not sorted by ID"
        );

        Ok(())
    }

    /// Creates a new account from [`NewAccountData`].
    ///
    /// This does not check serial uniqueness/ordering.
    pub fn create_new_account(
        &mut self,
        id: AccountId,
        serial: AccountSerial,
        new_acct_data: NewAccountData,
    ) -> StateResult<()> {
        let acct = OLAccountState::new_with_serial(new_acct_data, serial);
        self.create_account(id, acct)?;
        Ok(())
    }

    /// Calculates the total funds across all accounts in the ledger.
    pub(crate) fn calculate_total_funds(&self) -> BitcoinAmount {
        self.accounts
            .iter()
            .fold(BitcoinAmount::ZERO, |acc, entry| {
                acc.checked_add(entry.state.balance)
                    .expect("ol/state: total funds overflow")
            })
    }
}

impl TsnlAccountEntry {
    fn new(id: AccountId, state: OLAccountState) -> Self {
        Self { id, state }
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::{BitcoinAmount, SYSTEM_RESERVED_ACCTS};
    use strata_ledger_types::IAccountState;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::{
        ssz_generated::ssz::state::OLAccountTypeState,
        test_utils::{tsnl_account_entry_strategy, tsnl_ledger_accounts_table_strategy},
    };

    // Helper function to create an Empty account state
    fn create_empty_account_state(serial: AccountSerial, balance: BitcoinAmount) -> OLAccountState {
        OLAccountState::new(serial, balance, OLAccountTypeState::Empty)
    }

    // Helper function to create test account IDs
    fn test_account_id(n: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        AccountId::from(bytes)
    }

    #[test]
    fn test_create_single_account() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create an account
        let account_id = test_account_id(1);
        let serial = AccountSerial::zero();
        let balance = BitcoinAmount::from_sat(1000);
        let account_state = create_empty_account_state(serial, balance);

        // Add the account
        let result = table.create_account(account_id, account_state.clone());
        assert!(result.is_ok());

        // Verify the account was added
        assert_eq!(table.accounts.len(), 1);

        // Verify we can retrieve the account state
        let retrieved_state = table.get_account_state(&account_id);
        assert!(retrieved_state.is_some());
        assert_eq!(retrieved_state.unwrap().serial(), serial);
        assert_eq!(retrieved_state.unwrap().balance(), balance);
    }

    #[test]
    fn test_create_multiple_accounts_sorted_order() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create accounts in non-sorted order
        let account_ids = vec![
            test_account_id(3),
            test_account_id(1),
            test_account_id(5),
            test_account_id(2),
            test_account_id(4),
        ];

        let mut next_serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);

        for (i, account_id) in account_ids.iter().enumerate() {
            let serial = next_serial;
            next_serial = serial.incr();
            let balance = BitcoinAmount::from_sat((i as u64 + 1) * 100);
            let account_state = create_empty_account_state(serial, balance);

            let result = table.create_account(*account_id, account_state);
            assert!(result.is_ok(), "Failed to create account {}", i);
        }

        // Verify all accounts were added
        assert_eq!(table.accounts.len(), 5);

        // Verify accounts are sorted by ID
        for i in 1..table.accounts.len() {
            assert!(
                table.accounts[i - 1].id < table.accounts[i].id,
                "Accounts not sorted by ID"
            );
        }

        // Verify we can retrieve each account
        for account_id in &account_ids {
            let state = table.get_account_state(account_id);
            assert!(state.is_some(), "Could not find account {:?}", account_id);
        }
    }

    #[test]
    fn test_duplicate_account_id_rejected() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create first account
        let account_id = test_account_id(1);
        let serial1 = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
        let account_state1 = create_empty_account_state(serial1, BitcoinAmount::from_sat(1000));

        let result1 = table.create_account(account_id, account_state1);
        assert!(result1.is_ok());

        // Try to create account with same ID
        let serial2 = serial1.incr();
        let account_state2 = create_empty_account_state(serial2, BitcoinAmount::from_sat(2000));

        let result2 = table.create_account(account_id, account_state2);
        assert!(result2.is_err());

        match result2.unwrap_err() {
            StateError::AccountExists(id) => assert_eq!(id, account_id),
            _ => panic!("Expected AccountExists error"),
        }

        // Verify only one account exists
        assert_eq!(table.accounts.len(), 1);
    }

    #[test]
    fn test_get_account_state_mut() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create an account
        let account_id = test_account_id(1);
        let serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
        let initial_balance = BitcoinAmount::from_sat(1000);
        let account_state = create_empty_account_state(serial, initial_balance);

        table.create_account(account_id, account_state).unwrap();

        // Get mutable reference and modify balance
        {
            let state_mut = table.get_account_state_mut(&account_id);
            assert!(state_mut.is_some());

            let state = state_mut.unwrap();
            // We can't directly modify balance through the public API,
            // but we can verify we got a mutable reference
            assert_eq!(state.balance(), initial_balance);
        }

        // Verify the account still exists and is accessible
        let state = table.get_account_state(&account_id);
        assert!(state.is_some());
        assert_eq!(state.unwrap().balance(), initial_balance);
    }

    #[test]
    fn test_get_nonexistent_account() {
        let table = TsnlLedgerAccountsTable::new_empty();

        // Try to get a non-existent account
        let account_id = test_account_id(1);
        let state = table.get_account_state(&account_id);
        assert!(state.is_none());
    }

    #[test]
    fn test_ssz_roundtrip_empty_table() {
        let table = TsnlLedgerAccountsTable::new_empty();

        // Encode using SSZ
        let encoded = table.as_ssz_bytes();

        // Decode using SSZ
        let decoded =
            TsnlLedgerAccountsTable::from_ssz_bytes(&encoded).expect("Failed to decode table");

        // Verify they match
        assert_eq!(decoded.accounts.len(), table.accounts.len());
    }

    #[test]
    fn test_ssz_roundtrip_with_accounts() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Add several accounts
        let mut next_serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
        for i in 1..=5 {
            let account_id = test_account_id(i);
            let serial = next_serial;
            next_serial = serial.incr();
            let balance = BitcoinAmount::from_sat((i as u64) * 1000);
            let account_state = create_empty_account_state(serial, balance);

            table.create_account(account_id, account_state).unwrap();
        }

        // Encode using SSZ
        let encoded = table.as_ssz_bytes();

        // Decode using SSZ
        let decoded =
            TsnlLedgerAccountsTable::from_ssz_bytes(&encoded).expect("Failed to decode table");

        // Verify accounts match
        assert_eq!(decoded.accounts.len(), table.accounts.len());
        for i in 0..table.accounts.len() {
            assert_eq!(decoded.accounts[i].id, table.accounts[i].id);
            assert_eq!(
                decoded.accounts[i].state.serial(),
                table.accounts[i].state.serial()
            );
            assert_eq!(
                decoded.accounts[i].state.balance(),
                table.accounts[i].state.balance()
            );
        }
    }

    mod tsnl_account_entry {
        use super::*;

        ssz_proptest!(TsnlAccountEntry, tsnl_account_entry_strategy());
    }

    mod tsnl_ledger_accounts_table {
        use super::*;

        ssz_proptest!(
            TsnlLedgerAccountsTable,
            tsnl_ledger_accounts_table_strategy()
        );
    }
}
