//! Test utilities for state-support-types tests.

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MessageEntry, MsgPayload};
use strata_asm_manifest_types::AsmLogEntry;
use strata_identifiers::{AccountSerial, Epoch, L1BlockCommitment, L1Height, Slot};
use strata_ledger_types::{
    ISnarkAccountState, IStateAccessorMut, NewAccountData, NewAccountTypeState, PendingAsmLog,
};
use strata_ol_params::OLParams;
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;

use crate::memory_state_layer::MemoryStateBaseLayer;

/// Creates a genesis OLState using minimal empty parameters.
pub(crate) fn create_test_genesis_state() -> OLState {
    let params = OLParams::new_empty(L1BlockCommitment::default());
    OLState::from_genesis_params(&params).expect("valid params")
}

/// Creates a [`MemoryStateBaseLayer`] whose genesis header is at the given
/// epoch and slot.
pub(crate) fn new_layer_at(epoch: Epoch, slot: Slot) -> MemoryStateBaseLayer {
    let mut params = OLParams::new_empty(L1BlockCommitment::default());
    params.header.slot = slot;
    params.header.epoch = epoch;
    let state = OLState::from_genesis_params(&params)
        .expect("failed to create OLState from genesis params");
    MemoryStateBaseLayer::new(state)
}

/// Creates a [`PendingAsmLog`] whose height and payload byte are derived from
/// `tag`.
pub(crate) fn test_pending_asm_log(tag: u8) -> PendingAsmLog {
    let entry = AsmLogEntry::from_raw(vec![tag]).expect("bytes within capacity");
    PendingAsmLog::new(L1Height::from(tag as u32), entry)
}

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
    OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), test_hash(state_root_seed))
}

/// Create a test message entry for inbox testing.
pub(crate) fn test_message_entry(source_seed: u8, epoch: u32, value_sats: u64) -> MessageEntry {
    let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(value_sats), vec![source_seed])
        .expect("message payload bytes must fit within SSZ max length");
    MessageEntry::new(test_account_id(source_seed), epoch, payload)
}

/// Creates [`NewAccountData`] for a snark account from a test snark state and balance.
pub(crate) fn test_new_snark_account_data(
    snark_state: &OLSnarkAccountState,
    balance: BitcoinAmount,
) -> NewAccountData {
    NewAccountData::new(
        balance,
        NewAccountTypeState::Snark {
            update_vk: snark_state.update_vk().clone(),
            initial_state_root: snark_state.inner_state_root(),
        },
    )
}

/// Setup a [`MemoryStateBaseLayer`] with a snark account.
/// Returns (layer, account_serial).
pub(crate) fn setup_layer_with_snark_account(
    account_id: AccountId,
    state_root_seed: u8,
    initial_balance: BitcoinAmount,
) -> (MemoryStateBaseLayer, AccountSerial) {
    let mut layer = MemoryStateBaseLayer::new(create_test_genesis_state());
    let snark_state = test_snark_account_state(state_root_seed);
    let new_acct = test_new_snark_account_data(&snark_state, initial_balance);
    let serial = layer.create_new_account(account_id, new_acct).unwrap();
    (layer, serial)
}

/// Creates a [`MemoryStateBaseLayer`] from genesis.
pub(crate) fn create_test_base_layer() -> MemoryStateBaseLayer {
    MemoryStateBaseLayer::new(create_test_genesis_state())
}
