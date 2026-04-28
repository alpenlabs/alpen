//! Tests for multiple operations in a single update

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::{IAccountState, IStateAccessor};

use crate::{BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID, errors::ExecError, test_utils::*};

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
fn test_snark_update_multiple_output_messages() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create update with multiple output messages using SnarkUpdateBuilder
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 10_000_000, vec![1, 2, 3])
    .with_output_message(SEQUENCER_ACCT_ID, 5_000_000, vec![4, 5, 6])
    .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 0, vec![7, 8, 9])
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Multiple output messages should succeed: {:?}",
        result.err()
    );

    // Verify balance reduced by message values (10M + 5M + 0 = 15M)
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(85_000_000),
        "Balance should be reduced by total message value"
    );
}

#[test]
fn test_snark_update_transfers_and_messages_combined() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create update with both transfers and messages using SnarkUpdateBuilder
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 25_000_000)
    .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 15_000_000, vec![42, 43, 44])
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Combined transfers and messages should succeed: {:?}",
        result.err()
    );

    // Verify balances (100M - 25M - 15M = 60M)
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(60_000_000),
        "Sender should have 100M - 25M - 15M = 60M"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(25_000_000),
        "Recipient should receive 25M"
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
