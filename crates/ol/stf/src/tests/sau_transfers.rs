//! Tests for snark account transfer updates.

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};

use crate::{BRIDGE_GATEWAY_ACCT_ID, errors::ExecError, test_utils::*};

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
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, transfer_amount)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Valid update should succeed: {:?}",
        result.err()
    );

    // Verify balances
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(70_000_000),
        "Sender account balance should be 100M - 30M"
    );
    // Check the seq no of the sender
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
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
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient1_id, 30_000_000)
    .with_transfer(recipient2_id, 20_000_000)
    .with_transfer(recipient3_id, 10_000_000)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Multiple transfers should succeed: {:?}",
        result.err()
    );

    // Verify all balances
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(40_000_000),
        "Sender should have 100M - 60M = 40M"
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
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
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
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
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
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 0) // Zero value
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    // Should succeed - zero transfers are valid
    assert!(
        result.is_ok(),
        "Zero value transfer should succeed: {:?}",
        result.err()
    );

    // Verify balances unchanged
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
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
    let tx_nonzero = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
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
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        0,
        "Sequence number should not increment on failed transfer"
    );

    // Test case 2: Zero value transfer from zero balance account should succeed
    let tx_zero = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 0) // Zero value transfer
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let result2 = execute_tx_in_block(&mut state, genesis_block.header(), tx_zero, slot, epoch);

    // Zero transfer should succeed even from zero balance
    assert!(
        result2.is_ok(),
        "Zero value transfer from zero balance should succeed: {:?}",
        result2.err()
    );
    let blk2 = result2.unwrap();

    // Verify sequence number DID increment for successful zero transfer
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        1,
        "Sequence number should increment even for zero transfer"
    );
    assert_eq!(
        snark_account.balance(),
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
    let tx_multiple = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 0) // Zero transfer
    .with_transfer(snark_id, 0) // Self zero transfer
    .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 0, vec![]) // Zero value message
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let result3 = execute_tx_in_block(&mut state, blk2.header(), tx_multiple, slot + 1, epoch);

    // Multiple zero operations should all succeed
    assert!(
        result3.is_ok(),
        "Multiple zero operations from zero balance should succeed: {:?}",
        result3.err()
    );

    // Verify final state
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
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
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(snark_id, 30_000_000) // Transfer to self
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    assert!(
        result.is_ok(),
        "Self transfer should succeed: {:?}",
        result.err()
    );

    // Verify balance unchanged (sent 30M to self)
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after self-transfer"
    );
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        1,
        "Sequence number should increment"
    );
}

#[test]
fn test_snark_update_exact_balance_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Transfer exactly the entire balance
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 100_000_000) // Entire balance
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    assert!(
        result.is_ok(),
        "Exact balance transfer should succeed: {:?}",
        result.err()
    );

    // Verify balances
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Recipient should receive entire balance"
    );
}
