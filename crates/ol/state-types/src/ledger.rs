//! Accounts system types.
//!
//! This uses the "transitional" types described in the OL STF spec.

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, SYSTEM_RESERVED_ACCTS};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_derive::Codec;

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
        assert_eq!(
            acct_state.serial(),
            self.next_avail_serial(),
            "test: invalid serial sequencing"
        );

        if self.get_acct_entry_idx(&id).is_some() {
            return Err(AcctError::AccountIdExists(id));
        }

        let serial = self.next_avail_serial();
        let entry = TsnlAccountEntry::new(id, acct_state);
        self.accounts.push(entry);
        Ok(serial)
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

        Ok(Self {
            accounts,
            serials,
        })
    }
}
