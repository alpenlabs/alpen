use std::error::Error;

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};

/// Interface to the accounts ledger from the perspective of a single account.
///
/// This abstraction allows snark-acct-sys to apply update outputs without depending
/// on STF implementation details.
pub trait LedgerInterface {
    type Error: Error;

    /// Sends a value transfer to another account (no message data).
    fn send_transfer(&mut self, dest: AccountId, value: BitcoinAmount) -> Result<(), Self::Error>;

    /// Sends a message with attached value to another account.
    fn send_message(&mut self, dest: AccountId, payload: MsgPayload) -> Result<(), Self::Error>;
}
