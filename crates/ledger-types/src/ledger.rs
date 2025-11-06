use std::error::Error;

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};

/// Trait that exposes interface to the accounts ledger from the perspective of an account.
pub trait LedgerInterface<E: Error> {
    /// Send transfer to some account.
    fn send_transfer(&mut self, dest: AccountId, value: BitcoinAmount) -> Result<(), E>;

    /// Send message to some account.
    fn send_message(&mut self, dest: AccountId, payload: MsgPayload) -> Result<(), E>;
}
