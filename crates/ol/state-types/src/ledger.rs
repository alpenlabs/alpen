//! Accounts system types.
//!
//! This uses the "transitional" types described in the OL STF spec.

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, SYSTEM_RESERVED_ACCTS};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::account::AccountState;

/// Enshrined ledger accounts table.
///
/// This is part of a transitional design.
#[derive(Clone, Debug)]
pub struct TsnlLedgerAccountsTable {
    accounts: Vec<TsnlAccountEntry>,
    serials: Vec<AccountId>,
}

impl TsnlLedgerAccountsTable {
    /// Creates a new empty table.
    ///
    /// This reserves serials for system accounts with 0 values.
    pub fn new_empty() -> Self {
        Self {
            accounts: Vec::new(),
            serials: vec![AccountId::zero(); SYSTEM_RESERVED_ACCTS as usize],
        }
    }

    pub(crate) fn next_avail_serial(&self) -> AccountSerial {
        AccountSerial::from(self.serials.len() as u32)
    }

    fn get_acct_entry_idx(&self, id: &AccountId) -> Option<usize> {
        self.accounts.binary_search_by_key(id, |e| e.id).ok()
    }

    fn get_acct_entry(&self, id: &AccountId) -> Option<&TsnlAccountEntry> {
        let idx = self.get_acct_entry_idx(id)?;
        Some(&self.accounts[idx])
    }

    fn get_acct_entry_mut(&mut self, id: &AccountId) -> Option<&mut TsnlAccountEntry> {
        let idx = self.get_acct_entry_idx(id)?;
        Some(&mut self.accounts[idx])
    }

    pub(crate) fn get_account_state(&self, id: &AccountId) -> Option<&AccountState> {
        self.get_acct_entry(id).map(|e| &e.state)
    }

    pub(crate) fn get_account_state_mut(&mut self, id: &AccountId) -> Option<&mut AccountState> {
        self.get_acct_entry_mut(id).map(|e| &mut e.state)
    }

    /// Creates a new account.
    ///
    /// # Panics
    ///
    /// If the serial of the provided account doesn't match the value of
    /// `.next_avail_serial()` when called.
    pub(crate) fn create_account(
        &mut self,
        id: AccountId,
        acct_state: AccountState,
    ) -> AcctResult<AccountSerial> {
        // Sanity check, this should get optimized out.
        let next_serial = self.next_avail_serial();
        assert_eq!(
            acct_state.serial(),
            next_serial,
            "test: invalid serial sequencing"
        );

        // Figure out where we're supposed to put it.
        let insert_idx = match self.accounts.binary_search_by_key(&id, |e| e.id) {
            Ok(_) => return Err(AcctError::AccountIdExists(id)),
            Err(i) => i,
        };

        // Actually insert the entry.
        let entry = TsnlAccountEntry::new(id, acct_state);
        self.accounts.insert(insert_idx, entry);
        self.serials.push(id);

        // Sanity check.
        assert!(
            self.accounts.is_sorted_by_key(|e| e.id),
            "ol/state: accounts table not sorted by ID"
        );

        Ok(next_serial)
    }

    /// Gets the account ID corresponding to a serial.
    pub(crate) fn get_serial_acct_id(&self, serial: AccountSerial) -> Option<&AccountId> {
        self.serials.get(*serial.inner() as usize)
    }
}

#[derive(Clone, Debug, Codec)]
struct TsnlAccountEntry {
    id: AccountId,
    state: AccountState,
}

impl TsnlAccountEntry {
    fn new(id: AccountId, state: AccountState) -> Self {
        Self { id, state }
    }
}

// Codec implementation for TsnlLedgerAccountsTable
impl Codec for TsnlLedgerAccountsTable {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode the accounts vector - length first, then each element
        (self.accounts.len() as u64).encode(enc)?;
        for account in &self.accounts {
            account.encode(enc)?;
        }

        // Encode the serials vector - length first, then each element
        (self.serials.len() as u64).encode(enc)?;
        for serial in &self.serials {
            serial.encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode the accounts vector
        let accounts_len = u64::decode(dec)? as usize;
        let mut accounts = Vec::with_capacity(accounts_len);
        for _ in 0..accounts_len {
            accounts.push(TsnlAccountEntry::decode(dec)?);
        }

        // Decode the serials vector
        let serials_len = u64::decode(dec)? as usize;
        let mut serials = Vec::with_capacity(serials_len);
        for _ in 0..serials_len {
            serials.push(AccountId::decode(dec)?);
        }

        Ok(Self { accounts, serials })
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_ledger_types::IAccountState;

    use super::*;
    use crate::account::NativeAccountTypeState;

    // Helper function to create an Empty account state
    fn create_empty_account_state(serial: AccountSerial, balance: BitcoinAmount) -> AccountState {
        AccountState::new(serial, balance, NativeAccountTypeState::Empty)
    }

    // Helper function to create test account IDs
    fn test_account_id(n: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        AccountId::from(bytes)
    }

    #[test]
    fn test_new_empty_table() {
        let table = TsnlLedgerAccountsTable::new_empty();

        // Verify the table starts empty
        assert_eq!(table.accounts.len(), 0);

        // Verify system reserved accounts are initialized
        assert_eq!(table.serials.len(), SYSTEM_RESERVED_ACCTS as usize);

        // Verify all reserved serials are zero AccountIds
        for serial in &table.serials {
            assert_eq!(*serial, AccountId::zero());
        }

        // Verify next available serial is correct
        assert_eq!(
            table.next_avail_serial(),
            AccountSerial::from(SYSTEM_RESERVED_ACCTS)
        );
    }

    #[test]
    fn test_create_single_account() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create an account
        let account_id = test_account_id(1);
        let serial = table.next_avail_serial();
        let balance = BitcoinAmount::from_sat(1000);
        let account_state = create_empty_account_state(serial, balance);

        // Add the account
        let result = table.create_account(account_id, account_state.clone());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serial);

        // Verify the account was added
        assert_eq!(table.accounts.len(), 1);
        assert_eq!(table.serials.len(), (SYSTEM_RESERVED_ACCTS + 1) as usize);

        // Verify we can retrieve the account state
        let retrieved_state = table.get_account_state(&account_id);
        assert!(retrieved_state.is_some());
        assert_eq!(retrieved_state.unwrap().serial(), serial);
        assert_eq!(retrieved_state.unwrap().balance(), balance);

        // Verify the serial mapping
        let serial_account_id = table.get_serial_acct_id(serial);
        assert!(serial_account_id.is_some());
        assert_eq!(*serial_account_id.unwrap(), account_id);
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

        for (i, account_id) in account_ids.iter().enumerate() {
            let serial = table.next_avail_serial();
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
        let serial1 = table.next_avail_serial();
        let account_state1 = create_empty_account_state(serial1, BitcoinAmount::from_sat(1000));

        let result1 = table.create_account(account_id, account_state1);
        assert!(result1.is_ok());

        // Try to create account with same ID
        let serial2 = table.next_avail_serial();
        let account_state2 = create_empty_account_state(serial2, BitcoinAmount::from_sat(2000));

        let result2 = table.create_account(account_id, account_state2);
        assert!(result2.is_err());

        match result2.unwrap_err() {
            AcctError::AccountIdExists(id) => assert_eq!(id, account_id),
            _ => panic!("Expected AccountIdExists error"),
        }

        // Verify only one account exists
        assert_eq!(table.accounts.len(), 1);
    }

    #[test]
    fn test_get_account_state_mut() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create an account
        let account_id = test_account_id(1);
        let serial = table.next_avail_serial();
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

        // Try to get account ID for non-existent serial
        let serial = AccountSerial::from(1000);
        let serial_account_id = table.get_serial_acct_id(serial);
        assert!(serial_account_id.is_none());
    }

    #[test]
    fn test_serial_sequence() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Verify serials increase sequentially
        let mut expected_serial = SYSTEM_RESERVED_ACCTS;

        for i in 0..10 {
            let serial = table.next_avail_serial();
            assert_eq!(serial, AccountSerial::from(expected_serial));

            let account_id = test_account_id(i);
            let account_state =
                create_empty_account_state(serial, BitcoinAmount::from_sat(i as u64));

            table.create_account(account_id, account_state).unwrap();
            expected_serial += 1;
        }

        // Verify final serial is correct
        assert_eq!(
            table.next_avail_serial(),
            AccountSerial::from(expected_serial)
        );
    }

    #[test]
    #[should_panic(expected = "test: invalid serial sequencing")]
    fn test_invalid_serial_panics() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Create account with wrong serial (should panic)
        let account_id = test_account_id(1);
        let wrong_serial = AccountSerial::from(999); // Wrong serial
        let account_state = create_empty_account_state(wrong_serial, BitcoinAmount::from_sat(1000));

        // This should panic
        let _ = table.create_account(account_id, account_state);
    }

    #[test]
    fn test_codec_roundtrip_empty_table() {
        let table = TsnlLedgerAccountsTable::new_empty();

        // Encode
        let encoded = encode_to_vec(&table).expect("Failed to encode table");

        // Decode
        let decoded: TsnlLedgerAccountsTable =
            decode_buf_exact(&encoded).expect("Failed to decode table");

        // Verify they match
        assert_eq!(decoded.accounts.len(), table.accounts.len());
        assert_eq!(decoded.serials.len(), table.serials.len());

        for i in 0..table.serials.len() {
            assert_eq!(decoded.serials[i], table.serials[i]);
        }
    }

    #[test]
    fn test_codec_roundtrip_with_accounts() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Add several accounts
        for i in 1..=5 {
            let account_id = test_account_id(i);
            let serial = table.next_avail_serial();
            let balance = BitcoinAmount::from_sat((i as u64) * 1000);
            let account_state = create_empty_account_state(serial, balance);

            table.create_account(account_id, account_state).unwrap();
        }

        // Encode
        let encoded = encode_to_vec(&table).expect("Failed to encode table");

        // Decode
        let decoded: TsnlLedgerAccountsTable =
            decode_buf_exact(&encoded).expect("Failed to decode table");

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

        // Verify serials match
        assert_eq!(decoded.serials.len(), table.serials.len());
        for i in 0..table.serials.len() {
            assert_eq!(decoded.serials[i], table.serials[i]);
        }

        // Verify next serial is preserved
        assert_eq!(decoded.next_avail_serial(), table.next_avail_serial());
    }

    #[test]
    fn test_get_serial_acct_id_boundaries() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Test system reserved serials (should all be zero)
        for i in 0..SYSTEM_RESERVED_ACCTS {
            let serial = AccountSerial::from(i);
            let account_id = table.get_serial_acct_id(serial);
            assert!(account_id.is_some());
            assert_eq!(*account_id.unwrap(), AccountId::zero());
        }

        // Add an account
        let account_id = test_account_id(1);
        let serial = table.next_avail_serial();
        let account_state = create_empty_account_state(serial, BitcoinAmount::from_sat(1000));
        table.create_account(account_id, account_state).unwrap();

        // Test the new serial
        let retrieved_id = table.get_serial_acct_id(serial);
        assert!(retrieved_id.is_some());
        assert_eq!(*retrieved_id.unwrap(), account_id);

        // Test out-of-bounds serial
        let out_of_bounds = AccountSerial::from(1000);
        assert!(table.get_serial_acct_id(out_of_bounds).is_none());
    }

    #[test]
    fn test_binary_search_efficiency() {
        let mut table = TsnlLedgerAccountsTable::new_empty();

        // Add many accounts to test binary search
        for i in 0..100 {
            // Create account IDs that are not sequential but will be sorted
            let mut bytes = [0u8; 32];
            bytes[0] = (i * 2) as u8; // Even numbers to leave gaps
            let account_id = AccountId::from(bytes);

            let serial = table.next_avail_serial();
            let balance = BitcoinAmount::from_sat(i);
            let account_state = create_empty_account_state(serial, balance);

            table.create_account(account_id, account_state).unwrap();
        }

        // Verify all accounts can be found
        for i in 0..100 {
            let mut bytes = [0u8; 32];
            bytes[0] = (i * 2) as u8;
            let account_id = AccountId::from(bytes);

            let state = table.get_account_state(&account_id);
            assert!(state.is_some());
            assert_eq!(state.unwrap().balance(), BitcoinAmount::from_sat(i));
        }

        // Verify non-existent accounts (odd numbers) are not found
        for i in 0..100 {
            let mut bytes = [0u8; 32];
            bytes[0] = (i * 2 + 1) as u8; // Odd numbers that don't exist
            let account_id = AccountId::from(bytes);

            let state = table.get_account_state(&account_id);
            assert!(state.is_none());
        }
    }
}
