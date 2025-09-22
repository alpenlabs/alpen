use std::collections::HashMap;

use sha2::{Digest, Sha256};
use strata_primitives::buf::Buf32;
use thiserror::Error;

use crate::account::{
    AccountId, AccountInnerState, AccountSerial, AccountState, SnarkAccountMessageEntry,
};

#[derive(Debug, Error)]
pub enum LedgerError {}

pub type LedgerResult<T> = Result<T, LedgerError>;

/// Interface for accessing and modifying accounts ledger
pub trait LedgerProvider {
    /// Root of the current accounts ledger. For example root of the accounts trie.
    fn accounts_root(&self) -> LedgerResult<Buf32>;

    /// Get account id from serial
    fn get_account_id(&self, serial: AccountSerial) -> LedgerResult<Option<AccountId>>;

    /// Get an account state
    fn get_account_state(&self, acct_id: &AccountId) -> LedgerResult<Option<AccountState>>;

    /// Convenient method for accessing state via serial.
    fn get_account_state_by_serial(
        &self,
        serial: AccountSerial,
    ) -> LedgerResult<Option<AccountState>> {
        if let Some(acct_id) = self.get_account_id(serial)? {
            self.get_account_state(&acct_id)
        } else {
            Ok(None)
        }
    }

    /// Set an account state
    fn set_account_state(
        &mut self,
        acct_id: AccountId,
        acct_state: AccountState,
    ) -> LedgerResult<()>;

    /// Insert message to an account message mmr/queue.
    // TODO: message can be a bit generic instead of snark message?
    fn insert_message(
        &mut self,
        acct_id: &AccountId,
        message: SnarkAccountMessageEntry,
    ) -> LedgerResult<()>;
}

/// Simplest in-memory ledger. All it has is an in-memory map of acct id to list of messages.
#[derive(Debug, Clone)]
pub struct InMemoryVectorLedger {
    pub serial_to_id: HashMap<AccountSerial, AccountId>,
    pub account_states: HashMap<AccountId, AccountState>,
    pub root_cache: Option<Buf32>,
}

impl InMemoryVectorLedger {
    pub fn new() -> Self {
        Self {
            serial_to_id: HashMap::new(),
            account_states: HashMap::new(),
            root_cache: None,
        }
    }

    pub fn create_account(&mut self, serial: AccountSerial, id: AccountId, state: AccountState) {
        self.serial_to_id.insert(serial, id);
        self.account_states.insert(id, state);
        self.invalidate_root_cache();
    }

    fn invalidate_root_cache(&mut self) {
        self.root_cache = None;
    }

    pub fn compute_root(&mut self) -> LedgerResult<Buf32> {
        if let Some(cached) = &self.root_cache {
            return Ok(*cached);
        }

        let mut hasher = Sha256::new();

        // Sort account IDs for deterministic ordering
        let mut sorted_accounts: Vec<_> = self.account_states.keys().collect();
        sorted_accounts.sort();

        for account_id in sorted_accounts {
            hasher.update(account_id.as_ref());
            if let Some(state) = self.account_states.get(account_id) {
                hasher.update(state.serial.to_be_bytes());
                hasher.update(state.ty.to_be_bytes());
                hasher.update(state.balance.to_be_bytes());
            }
        }

        let result = Buf32::new(hasher.finalize().into());
        self.root_cache = Some(result);
        Ok(result)
    }
}

impl Default for InMemoryVectorLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerProvider for InMemoryVectorLedger {
    fn accounts_root(&self) -> LedgerResult<Buf32> {
        // Need mutable access to compute/cache root, but trait requires &self
        // For now, recompute every time (inefficient but correct)
        let mut hasher = Sha256::new();

        let mut sorted_accounts: Vec<_> = self.account_states.iter().collect();
        sorted_accounts.sort_by_key(|(k, _)| **k);

        for (acct_id, state) in sorted_accounts {
            hasher.update(acct_id.as_slice());
            hasher.update(state.serial.to_be_bytes());
            hasher.update(state.ty.to_be_bytes());
            hasher.update(state.balance.to_be_bytes());
        }

        Ok(Buf32::new(hasher.finalize().into()))
    }

    fn get_account_id(&self, serial: AccountSerial) -> LedgerResult<Option<AccountId>> {
        Ok(self.serial_to_id.get(&serial).copied())
    }

    fn get_account_state(&self, acct_id: &AccountId) -> LedgerResult<Option<AccountState>> {
        Ok(self.account_states.get(acct_id).cloned())
    }

    fn insert_message(
        &mut self,
        acct_id: &AccountId,
        message: SnarkAccountMessageEntry,
    ) -> LedgerResult<()> {
        if let Some(AccountInnerState::Snark(mut acct)) =
            self.get_account_state(acct_id)?.map(|a| a.inner_state)
        {
            // TODO: Will this actually update the hashmap?
            acct.input.push(message);
            self.invalidate_root_cache();
        }
        Ok(())
    }

    fn set_account_state(
        &mut self,
        acct_id: AccountId,
        acct_state: AccountState,
    ) -> LedgerResult<()> {
        self.account_states.insert(acct_id, acct_state);
        self.invalidate_root_cache();
        Ok(())
    }
}
