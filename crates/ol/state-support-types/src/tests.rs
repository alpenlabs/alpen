//! Integration tests for combined state layers.
//!
//! These tests verify that multiple wrapper layers can be composed together
//! and work correctly.

use strata_acct_types::BitcoinAmount;
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, L1BlockId, L1Height, WtxidsRoot};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, IAccountStateMut, ISnarkAccountState,
    ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_state_types::{OLState, WriteBatch};

use crate::{BatchDiffState, IndexerState, WriteTrackingState, test_utils::*};

// =============================================================================
// IndexerState over WriteTrackingState tests
// =============================================================================

/// Test that IndexerState can wrap WriteTrackingState and both function correctly.
#[test]
fn test_indexer_over_write_tracking_basic() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create the layer stack: IndexerState<WriteTrackingState<&OLState>>
    let batch = WriteBatch::new_from_state(&base_state);
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
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    let batch = WriteBatch::new_from_state(&base_state);
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
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

/// Test manifest tracking through combined layers.
#[test]
fn test_combined_manifest_tracking() {
    let base_state = OLState::new_genesis();
    let batch = WriteBatch::new_from_state(&base_state);
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
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    let batch = WriteBatch::new_from_state(&base_state);
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
    let batch = WriteBatch::new_from_state(&base_state);
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
    let batch = WriteBatch::new_from_state(&base_state);
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
    let (base_state, _) =
        setup_state_with_snark_account(account_id_1, 1, BitcoinAmount::from_sat(1000));

    let batch = WriteBatch::new_from_state(&base_state);
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
// WriteTrackingState over BatchDiffState tests
// =============================================================================

/// Test that WriteTrackingState can wrap BatchDiffState and all write operations work correctly.
/// This verifies that we can build on top of a read-only diff layer with pending batches.
#[test]
fn test_write_tracking_over_batch_diff_basic() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create a pending batch with some modifications
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    pending_batch.global_mut().set_cur_slot(50);
    pending_batch.epochal_mut().set_cur_epoch(3);

    // Create BatchDiffState with the pending batch
    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top of BatchDiffState
    // The write batch needs to be initialized with values from the diff state
    // (WriteTrackingState reads global/epochal from its own batch, not from base)
    let mut write_batch = WriteBatch::new_from_state(&base_state);
    write_batch.global_mut().set_cur_slot(diff_state.cur_slot());
    write_batch
        .epochal_mut()
        .set_cur_epoch(diff_state.cur_epoch());
    let tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Verify we can read through the layers (account from base via diff_state)
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));

    // Global/epochal come from the write batch (which we initialized from diff_state)
    assert_eq!(tracking.cur_slot(), 50);
    assert_eq!(tracking.cur_epoch(), 3);
}

/// Test that update_account works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_update_account() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create BatchDiffState (empty batches = pure passthrough)
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Update account balance
    tracking
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Verify the update worked
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));

    // Verify it's in the write batch
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
    assert_eq!(
        batch.ledger().get_account(&account_id).unwrap().balance(),
        BitcoinAmount::from_sat(1500)
    );
}

/// Test that create_new_account works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_create_account() {
    let base_state = OLState::new_genesis();

    // Create BatchDiffState with empty batches
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Create a new account
    let account_id = test_account_id(1);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(5000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = tracking.create_new_account(account_id, new_acct).unwrap();

    // Verify the account exists
    assert!(tracking.check_account_exists(account_id).unwrap());
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));

    // Verify it's in the write batch
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
}

/// Test that global/epochal setters work through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_global_epochal_setters() {
    let base_state = OLState::new_genesis();

    // Create BatchDiffState with a pending batch that has slot=50, epoch=3
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    pending_batch.global_mut().set_cur_slot(50);
    pending_batch.epochal_mut().set_cur_epoch(3);
    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Modify slot and epoch through WriteTrackingState
    tracking.set_cur_slot(100);
    tracking.set_cur_epoch(10);

    // Verify the values are updated
    assert_eq!(tracking.cur_slot(), 100);
    assert_eq!(tracking.cur_epoch(), 10);

    // Verify they're in the write batch
    let batch = tracking.into_batch();
    assert_eq!(batch.global().get_cur_slot(), 100);
    assert_eq!(batch.epochal().cur_epoch(), 10);
}

/// Test that inbox message insertion works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_inbox_message() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create BatchDiffState with empty batches
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Insert an inbox message
    let msg = test_message_entry(50, 0, 2000);
    tracking
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg.clone())
        })
        .unwrap()
        .unwrap();

    // Verify the message was inserted
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        1
    );

    // Verify base is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

/// Test reading account from pending batch through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_reads_from_pending_batch() {
    let base_state = OLState::new_genesis();

    // Create a pending batch with a new account
    let account_id_in_batch = test_account_id(1);
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(3000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = base_state.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id_in_batch, new_acct, serial);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Should be able to read the account from the pending batch
    assert!(tracking.check_account_exists(account_id_in_batch).unwrap());
    let account = tracking
        .get_account_state(account_id_in_batch)
        .unwrap()
        .unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(3000));
}

/// Test that WriteTrackingState over BatchDiffState can update an account from the pending batch.
#[test]
fn test_write_tracking_over_batch_diff_update_account_from_pending_batch() {
    let base_state = OLState::new_genesis();

    // Create a pending batch with a new account
    let account_id = test_account_id(1);
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(3000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = base_state.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id, new_acct, serial);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Update the account (copy-on-write from pending batch to write batch)
    tracking
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Verify the update worked
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(3500));

    // Verify it's now in the write batch with the updated balance
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
    assert_eq!(
        batch.ledger().get_account(&account_id).unwrap().balance(),
        BitcoinAmount::from_sat(3500)
    );
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

    let batch = WriteBatch::new_from_state(&base_state);
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
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        original_inbox_count
    );
}
