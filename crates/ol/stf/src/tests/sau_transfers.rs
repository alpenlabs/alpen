//! Tests for snark account transfer updates.

use strata_acct_types::{AcctError, BitcoinAmount, TxEffects};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, assembly::BlockComponents, context::BlockInfo, errors::ExecError,
    test_utils::*,
};

#[test]
fn test_snark_update_success_with_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create valid update with transfer
    let transfer_amount = 30_000_000u64;
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, transfer_amount)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let mut verify_state = state.clone();
    let (slot, epoch) = (1, 1);
    let block1 = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Valid update should succeed");

    assert_verification_succeeds(
        &mut verify_state,
        block1.header(),
        Some(genesis_block.header().clone()),
        block1.body(),
    );

    // Verify balances
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(70_000_000),
        "Sender account balance should be 100M - 30M"
    );
    // Check the seq no of the sender
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient should receive 30M"
    );
}

#[test]
fn test_snark_update_multiple_transfers() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(200);
    let recipient2_id = test_account_id(201);
    let recipient3_id = test_account_id(202);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient accounts
    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);
    create_empty_account(&mut state, recipient3_id);

    // Create update with multiple transfers (30M + 20M + 10M = 60M total)
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient1_id, 30_000_000)
        .with_transfer(recipient2_id, 20_000_000)
        .with_transfer(recipient3_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Multiple transfers should succeed");

    // Verify all balances
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(40_000_000),
        "Sender should have 100M - 60M = 40M"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient1 should receive 30M"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive 20M"
    );

    let recipient3 = state.get_account_state(recipient3_id).unwrap().unwrap();
    assert_eq!(
        recipient3.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient3 should receive 10M"
    );
}

#[test]
fn test_snark_update_same_block_distinct_senders() {
    let mut state = create_test_genesis_state();
    let sender1_id = get_test_snark_account_id();
    let sender2_id = test_account_id(TEST_SNARK_ACCOUNT_ID + 1);
    let recipient1_id = test_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = test_account_id(TEST_RECIPIENT_ID + 2);

    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);

    let genesis_block = setup_genesis_with_snark_accounts(
        &mut state,
        &[(sender1_id, 100_000_000), (sender2_id, 70_000_000)],
    );

    let sender1_state = lookup_snark_state(&state, sender1_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(sender1_state.clone())
        .with_transfer(recipient1_id, 10_000_000)
        .build(sender1_id, get_test_state_root(2), get_test_proof(1));

    let sender2_state = lookup_snark_state(&state, sender2_id);
    let tx2 = SnarkUpdateBuilder::from_snark_state(sender2_state.clone())
        .with_transfer(recipient2_id, 20_000_000)
        .build(sender2_id, get_test_state_root(3), get_test_proof(2));

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx1, tx2]),
    )
    .expect("Same-block updates from distinct senders should succeed");

    let (sender1_balance, sender1_state) = lookup_snark_account_states(&state, sender1_id);
    assert_eq!(
        sender1_balance.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "Sender1 balance should reflect its same-block transfer"
    );
    assert_eq!(
        *sender1_state.seqno().inner(),
        1,
        "Sender1 sequence number should increment"
    );

    let (sender2_balance, sender2_state) = lookup_snark_account_states(&state, sender2_id);
    assert_eq!(
        sender2_balance.balance(),
        BitcoinAmount::from_sat(50_000_000),
        "Sender2 balance should reflect its same-block transfer"
    );
    assert_eq!(
        *sender2_state.seqno().inner(),
        1,
        "Sender2 sequence number should increment"
    );

    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive sender1 transfer"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive sender2 transfer"
    );
}

#[test]
fn test_snark_update_same_block_sequential_seqnos() {
    let mut state = create_test_genesis_state();
    let sender_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = test_account_id(TEST_RECIPIENT_ID + 2);

    let genesis_block = setup_genesis_with_snark_account(&mut state, sender_id, 100_000_000);
    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);

    let sender_state = lookup_snark_state(&state, sender_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(sender_state.clone())
        .with_transfer(recipient1_id, 10_000_000)
        .build(sender_id, get_test_state_root(2), get_test_proof(1));

    let mut tx2_effects = TxEffects::default();
    tx2_effects.push_transfer(recipient2_id, 20_000_000);
    let tx2 = create_unchecked_snark_update(sender_id, 1, get_test_state_root(3), 0, tx2_effects);

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx1, tx2]),
    )
    .expect("Same-block sequential updates from one sender should succeed");

    let (sender_balance, sender_state) = lookup_snark_account_states(&state, sender_id);
    assert_eq!(
        sender_balance.balance(),
        BitcoinAmount::from_sat(70_000_000),
        "Sender balance should include both same-block transfers"
    );
    assert_eq!(
        *sender_state.seqno().inner(),
        2,
        "Sender sequence number should increment for both same-block updates"
    );

    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive the first transfer"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive the second transfer"
    );
}

#[test]
fn test_snark_update_same_block_duplicate_seqno_fails() {
    let mut state = create_test_genesis_state();
    let sender_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = test_account_id(TEST_RECIPIENT_ID + 2);

    let genesis_block = setup_genesis_with_snark_account(&mut state, sender_id, 100_000_000);
    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);

    let sender_state = lookup_snark_state(&state, sender_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(sender_state.clone())
        .with_transfer(recipient1_id, 10_000_000)
        .build(sender_id, get_test_state_root(2), get_test_proof(1));

    let mut tx2_effects = TxEffects::default();
    tx2_effects.push_transfer(recipient2_id, 20_000_000);
    let tx2 = create_unchecked_snark_update(sender_id, 0, get_test_state_root(3), 0, tx2_effects);

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let result = execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx1, tx2]),
    );

    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidUpdateSequence {
                account_id,
                expected,
                got,
            }) => {
                assert_eq!(account_id, sender_id);
                assert_eq!(expected, 1);
                assert_eq!(got, 0);
            }
            err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
        },
        Ok(_) => panic!("Same-block duplicate sequence number should fail"),
    }

    let (sender_balance, sender_state) = lookup_snark_account_states(&state, sender_id);
    assert_eq!(
        sender_balance.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "First same-block transfer should remain applied"
    );
    assert_eq!(
        *sender_state.seqno().inner(),
        1,
        "Sender sequence number should reflect only the first update"
    );

    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive the first transfer"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient2 should not receive the duplicate-seqno transfer"
    );
}

#[test]
fn test_snark_update_partial_balance_multiple_outputs() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(200);
    let recipient2_id = test_account_id(201);

    // Setup: genesis with snark account with 100M sats
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient accounts
    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);

    // Try to send 60M + 50M = 110M (exceeds balance of 100M)
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient1_id, 60_000_000)
        .with_transfer(recipient2_id, 50_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    assert!(result.is_err(), "Update exceeding balance should fail");
    match result.unwrap_err().into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    // Verify no partial execution - all balances should be unchanged
    let (ol_account_state, _) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );

    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient1 should have no balance"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient2 should have no balance"
    );
}

#[test]
fn test_snark_update_zero_value_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create update with zero value transfer
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 0) // Zero value
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Zero value transfer should succeed");

    // Zero transfers are valid.

    // Verify balances unchanged
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should still increment"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient balance should remain 0"
    );
}

#[test]
fn test_snark_update_from_zero_balance_account() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account that has ZERO balance
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 0);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Test case 1: Try to transfer non-zero amount from zero balance account
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx_nonzero = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 1) // Even 1 satoshi should fail
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx_nonzero, slot, epoch);

    // Should fail due to insufficient balance
    assert!(result.is_err(), "Transfer from zero balance should fail");

    match result.unwrap_err().into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    // Verify sequence number did NOT increment due to failure
    let snark_account_state = lookup_snark_state(&state, snark_id);
    assert_eq!(
        *snark_account_state.seqno().inner(),
        0,
        "Sequence number should not increment on failed transfer"
    );

    // Test case 2: Zero value transfer from zero balance account should succeed
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx_zero = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 0) // Zero value transfer
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let blk2 = execute_tx_in_block(&mut state, genesis_block.header(), tx_zero, slot, epoch)
        .expect("Zero value transfer from zero balance should succeed");

    // Verify sequence number DID increment for successful zero transfer
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should increment even for zero transfer"
    );
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(0),
        "Balance should remain zero"
    );

    // Verify recipient still has zero balance
    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient should have zero balance"
    );

    // Test case 3: Try multiple transfers from zero balance account
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx_multiple = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 0) // Zero transfer
        .with_transfer(snark_id, 0) // Self zero transfer
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 0, vec![]) // Zero value message
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    execute_tx_in_block(&mut state, blk2.header(), tx_multiple, slot + 1, epoch)
        .expect("Multiple zero operations from zero balance should succeed");

    // Verify final state
    let snark_account_state = lookup_snark_state(&state, snark_id);
    assert_eq!(
        *snark_account_state.seqno().inner(),
        2,
        "Sequence number should increment to 2"
    );
}

#[test]
fn test_snark_update_self_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create update transferring to self
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(snark_id, 30_000_000) // Transfer to self
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Self transfer should succeed");

    // Verify balance unchanged (sent 30M to self)
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after self-transfer"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should increment"
    );
}

#[test]
fn test_snark_update_exact_balance_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    let second_recipient_id = test_account_id(TEST_RECIPIENT_ID + 1);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);
    create_empty_account(&mut state, second_recipient_id);

    // Transfer exactly the entire balance
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 100_000_000) // Entire balance
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let block1 = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Exact balance transfer should succeed");

    // Verify balances
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should increment"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Recipient should receive entire balance"
    );

    let result = attempt_transfer_after_balance_drained(
        &mut state,
        block1.header(),
        snark_id,
        second_recipient_id,
        1,
        slot + 1,
        epoch,
    );
    match result {
        Err(e) => assert!(
            matches!(e.into_base(), ExecError::BalanceUnderflow),
            "Expected BalanceUnderflow"
        ),
        Ok(_) => panic!("Transfer after balance is drained should fail"),
    }

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(0),
        "Sender balance should remain zero after failed transfer from drained balance"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should not increment after failed transfer from drained balance"
    );

    let second_recipient = state
        .get_account_state(second_recipient_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        second_recipient.balance(),
        BitcoinAmount::from_sat(0),
        "Second recipient should not receive failed transfer from drained balance"
    );
}
