use strata_primitives::buf::Buf32;
use thiserror::Error;

use crate::account::{AccountId, AccountSerial, AccountState, SnarkAccountMessageEntry};

#[derive(Debug, Error)]
pub enum LedgerError {}

pub type LedgerResult<T> = Result<T, LedgerError>;

/// Interface for accessing and modifying accounts ledger
pub trait LedgerProvider {
    /// Root of the current accounts ledger. For example root of the accounts trie.
    fn root(&self) -> Buf32;

    /// Get account id from serial
    fn account_id(&self, serial: AccountSerial) -> LedgerResult<Option<AccountId>>;

    /// Get an account state
    fn account_state(&self, acct_id: AccountId) -> LedgerResult<Option<AccountState>>;

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
}
