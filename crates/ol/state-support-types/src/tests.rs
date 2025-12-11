//! Integration tests for combined state layers.
//!
//! These tests verify that multiple wrapper layers can be composed together
//! and work correctly.

use bitcoin::absolute;
use strata_acct_types::{BitcoinAmount, SYSTEM_RESERVED_ACCTS};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{
    AccountSerial, Buf32, L1BlockCommitment, L1BlockId, L1Height, WtxidsRoot,
};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, IAccountStateMut, ISnarkAccountState,
    ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_state_types::{EpochalState, GlobalState, OLState};

use crate::test_utils::*;
use crate::write_batch::WriteBatch;
use crate::{IndexerState, WriteTrackingState};

/// Helper to create a WriteBatch initialized from a base OLState.
fn create_batch_from_state(
    state: &OLState,
) -> WriteBatch<<OLState as IStateAccessor>::AccountState> {
    let epochal = EpochalState::new(
        state.total_ledger_balance(),
        state.cur_epoch(),
        L1BlockCommitment::new(
            absolute::Height::from_consensus(state.last_l1_height().into()).unwrap(),
            *state.last_l1_blkid(),
        ),
        state.asm_recorded_epoch().clone(),
    );
    let global = GlobalState::new(state.cur_slot());

    let mut batch = WriteBatch::new(global, epochal);
    let base_next_serial = AccountSerial::from(SYSTEM_RESERVED_ACCTS);
    batch.ledger_mut().set_next_serial(base_next_serial);

    batch
}

// =============================================================================
// IndexerState over WriteTrackingState tests
// =============================================================================

/// Test that IndexerState can wrap WriteTrackingState and both function correctly.
#[test]
fn test_indexer_over_write_tracking_basic() {
    let account_id = test_account_id(1);
    let (base_state, _serial) = setup_state_with_snark_account(
        account_id,
        1,
        BitcoinAmount::from_sat(1000),
    );

    // Create the layer stack: IndexerState<WriteTrackingState<&OLState>>
    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let indexer = IndexerState::new(tracking);

    // Verify we can read through both layers
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
}

/// Test inbox message tracking through both layers.
#[test]
fn test_combined_inbox_message_tracking() {
    let account_id = test_account_id(1);
    let (base_state, _serial) = setup_state_with_snark_account(
        account_id,
        1,
        BitcoinAmount::from_sat(1000),
    );

    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Insert an inbox message through the combined stack
    let msg = test_message_entry(50, 0, 2000);
    indexer
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg.clone())
        })
        .unwrap()
        .unwrap();

    // Extract the layers
    let (tracking, indexer_writes) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify IndexerState captured the inbox write
    assert_eq!(indexer_writes.inbox_messages().len(), 1);
    assert_eq!(indexer_writes.inbox_messages()[0].account_id, account_id);
    assert_eq!(indexer_writes.inbox_messages()[0].index, 0);

    // Verify WriteTrackingState has the modified account in the batch
    assert!(batch.ledger().contains_account(&account_id));

    // Verify base state is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account.as_snark_account().unwrap().inbox_mmr().num_entries(),
        0
    );
}

/// Test manifest tracking through combined layers.
#[test]
fn test_combined_manifest_tracking() {
    let base_state = OLState::new_genesis();
    let batch = create_batch_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Append a manifest through the combined stack
    let height = L1Height::from(100u32);
    let l1_blkid = L1BlockId::from(Buf32::from([1u8; 32]));
    let wtxids_root = WtxidsRoot::from(Buf32::from([2u8; 32]));
    let manifest = AsmManifest::new(l1_blkid, wtxids_root, vec![]);

    indexer.append_manifest(height, manifest);

    // Verify IndexerState captured the manifest write
    let (_, indexer_writes) = indexer.into_parts();
    assert_eq!(indexer_writes.manifests().len(), 1);
    assert_eq!(indexer_writes.manifests()[0].height, height);
}

/// Test balance modifications through combined layers.
#[test]
fn test_combined_balance_modification() {
    let account_id = test_account_id(1);
    let (base_state, _serial) = setup_state_with_snark_account(
        account_id,
        1,
        BitcoinAmount::from_sat(1000),
    );

    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Modify balance through the combined stack
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Extract and verify
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify the account is in the batch with updated balance
    let batch_account = batch.ledger().get_account(&account_id).unwrap();
    assert_eq!(batch_account.balance(), BitcoinAmount::from_sat(1500));

    // Verify base state is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(base_account.balance(), BitcoinAmount::from_sat(1000));
}

/// Test account creation through combined layers.
#[test]
fn test_combined_account_creation() {
    let base_state = OLState::new_genesis();
    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account through the combined stack
    let account_id = test_account_id(1);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(5000),
        AccountTypeState::Snark(snark_state),
    );

    let serial = indexer.create_new_account(account_id, new_acct).unwrap();

    // Verify the account exists through the stack
    assert!(indexer.check_account_exists(account_id).unwrap());
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));

    // Extract and verify it's in the batch
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
}

/// Test global state modifications through combined layers.
#[test]
fn test_combined_global_state_modification() {
    let base_state = OLState::new_genesis();
    let batch = create_batch_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Modify slot through the combined stack
    indexer.set_cur_slot(42);
    assert_eq!(indexer.cur_slot(), 42);

    // Modify epoch
    indexer.set_cur_epoch(5);
    assert_eq!(indexer.cur_epoch(), 5);

    // Extract and verify
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();

    assert_eq!(batch.global().get_cur_slot(), 42);
    assert_eq!(batch.epochal().cur_epoch(), 5);
}

/// Test multiple operations through combined layers.
#[test]
fn test_combined_multiple_operations() {
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);

    // Setup base state with one account
    let (base_state, _) = setup_state_with_snark_account(
        account_id_1,
        1,
        BitcoinAmount::from_sat(1000),
    );

    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account
    let snark_state_2 = test_snark_account_state(2);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(2000),
        AccountTypeState::Snark(snark_state_2),
    );
    indexer.create_new_account(account_id_2, new_acct).unwrap();

    // Insert messages to both accounts
    let msg1 = test_message_entry(10, 0, 1000);
    indexer
        .update_account(account_id_1, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg1.clone())
        })
        .unwrap()
        .unwrap();

    let msg2 = test_message_entry(20, 0, 2000);
    indexer
        .update_account(account_id_2, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg2.clone())
        })
        .unwrap()
        .unwrap();

    // Modify slot
    indexer.set_cur_slot(100);

    // Extract and verify all changes
    let (tracking, indexer_writes) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify IndexerState tracked both inbox writes
    assert_eq!(indexer_writes.inbox_messages().len(), 2);

    // Verify batch has both accounts
    assert!(batch.ledger().contains_account(&account_id_1));
    assert!(batch.ledger().contains_account(&account_id_2));

    // Verify slot was updated
    assert_eq!(batch.global().get_cur_slot(), 100);
}

// =============================================================================
// Tests verifying layer isolation
// =============================================================================

/// Test that modifications through combined layers don't affect the base state.
#[test]
fn test_combined_layers_preserve_base_state() {
    let account_id = test_account_id(1);
    let initial_balance = BitcoinAmount::from_sat(1000);
    let (base_state, _) = setup_state_with_snark_account(account_id, 1, initial_balance);

    // Save original values
    let original_slot = base_state.cur_slot();
    let original_epoch = base_state.cur_epoch();
    let original_inbox_count = base_state
        .get_account_state(account_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .inbox_mmr()
        .num_entries();

    let mut batch = create_batch_from_state(&base_state);
    batch
        .ledger_mut()
        .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Make various modifications
    indexer.set_cur_slot(999);
    indexer.set_cur_epoch(99);
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(test_message_entry(1, 0, 1000))
                .unwrap();
        })
        .unwrap();

    // Discard the layers (don't apply to base)
    drop(indexer);

    // Verify base state is completely unchanged
    assert_eq!(base_state.cur_slot(), original_slot);
    assert_eq!(base_state.cur_epoch(), original_epoch);

    let account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), initial_balance);
    assert_eq!(
        account.as_snark_account().unwrap().inbox_mmr().num_entries(),
        original_inbox_count
    );
}
