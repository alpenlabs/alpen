use std::collections::HashMap;

use strata_primitives::buf::Buf32;
use thiserror::Error;

use crate::account::{AccountId, AccountSerial, AccountState, SnarkAccountMessageEntry};

#[derive(Debug, Error)]
pub enum LedgerError {}

pub type LedgerResult<T> = Result<T, LedgerError>;

/// Interface for accessing and modifying accounts ledger
pub trait LedgerProvider {
    /// Root of the current accounts ledger. For example root of the accounts trie.
    fn root(&self) -> LedgerResult<Buf32>;

    /// Get account id from serial
    fn account_id(&self, serial: AccountSerial) -> LedgerResult<Option<AccountId>>;

    /// Get an account state
    fn account_state(&self, acct_id: &AccountId) -> LedgerResult<Option<AccountState>>;

    /// Convenient method for accessing state via serial.
    fn account_serial_state(&self, serial: AccountSerial) -> LedgerResult<Option<AccountState>> {
        if let Some(acct_id) = self.account_id(serial)? {
            self.account_state(&acct_id)
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

    /// insert message to an account message mmr/queue.
    // TODO: message can be a bit generic instead of snark message?
    fn insert_message(
        &mut self,
        acct_id: AccountId,
        message: SnarkAccountMessageEntry,
    ) -> LedgerResult<()>;

    /// Consume input messages. Most likely updates some input index in state
    fn consume_messages(
        &mut self,
        acct_id: AccountId,
        from_idx: u64,
        to_idx: u64,
    ) -> LedgerResult<()>;
}

/// Simplest in-memory ledger. All it has is an in-memory mapc of acct id to list of messages.
#[derive(Debug, Clone)]
pub struct InMemoryVectorLedger {
    pub acct_msgs: HashMap<AccountId, Vec<SnarkAccountMessageEntry>>,
}

impl LedgerProvider for InMemoryVectorLedger {
    fn root(&self) -> LedgerResult<Buf32> {
        // TODO: probably concat all and hash
        Ok(Buf32::zero())
    }

    fn account_id(&self, serial: AccountSerial) -> LedgerResult<Option<AccountId>> {
        todo!()
    }

    fn account_state(&self, acct_id: &AccountId) -> LedgerResult<Option<AccountState>> {
        todo!()
    }

    fn insert_message(
        &mut self,
        acct_id: AccountId,
        message: SnarkAccountMessageEntry,
    ) -> LedgerResult<()> {
        todo!()
    }

    fn consume_messages(
        &mut self,
        acct_id: AccountId,
        from_idx: u64,
        to_idx: u64,
    ) -> LedgerResult<()> {
        todo!()
    }

    fn set_account_state(
        &mut self,
        acct_id: AccountId,
        acct_state: AccountState,
    ) -> LedgerResult<()> {
        todo!()
    }
}
