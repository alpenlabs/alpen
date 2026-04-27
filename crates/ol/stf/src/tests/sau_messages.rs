//! Tests for snark account output-message updates.

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::{IAccountState, IStateAccessor};

use crate::{BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID, test_utils::*};

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
