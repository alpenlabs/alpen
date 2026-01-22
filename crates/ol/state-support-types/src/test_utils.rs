//! Test utilities for state-support-types tests.

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_identifiers::AccountSerial;
use strata_ledger_types::{AccountTypeState, IStateAccessor, NewAccountData};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::MessageEntry;

/// Create a test AccountId from a seed byte.
pub(crate) fn test_account_id(seed: u8) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    AccountId::from(bytes)
}

/// Create a test Hash from a seed byte.
pub(crate) fn test_hash(seed: u8) -> Hash {
    Hash::from([seed; 32])
}

/// Create a fresh snark account state for testing.
pub(crate) fn test_snark_account_state(state_root_seed: u8) -> OLSnarkAccountState {
    OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), test_hash(state_root_seed), Vec::new())
}

/// Create a test message entry for inbox testing.
pub(crate) fn test_message_entry(source_seed: u8, epoch: u32, value_sats: u64) -> MessageEntry {
    let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), vec![source_seed]);
    MessageEntry::new(test_account_id(source_seed), epoch, payload)
}

/// Setup an OLState with a snark account.
/// Returns (state, account_serial).
pub(crate) fn setup_state_with_snark_account(
    account_id: AccountId,
    state_root_seed: u8,
    initial_balance: BitcoinAmount,
) -> (OLState, AccountSerial) {
    let mut state = OLState::new_genesis();
    let snark_state = test_snark_account_state(state_root_seed);
    let new_acct = NewAccountData::new(initial_balance, AccountTypeState::Snark(snark_state));
    let serial = state.create_new_account(account_id, new_acct).unwrap();
    (state, serial)
}
