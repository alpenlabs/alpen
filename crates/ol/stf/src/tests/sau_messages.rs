//! Tests for snark account output-message updates.

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_state_types::OLState;

use crate::{BRIDGE_GATEWAY_ACCT_ID, test_utils::*};

#[test]
fn test_snark_update_multiple_output_messages() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(300);
    let recipient2_id = test_account_id(301);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    create_snark_account_with_balance(&mut state, recipient1_id, 0);
    create_snark_account_with_balance(&mut state, recipient2_id, 0);

    // Create update with multiple output messages using SnarkUpdateBuilder
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_output_message(recipient1_id, 10_000_000, vec![1, 2, 3])
        .with_output_message(recipient2_id, 5_000_000, vec![4, 5, 6])
        // Bridge-gateway non-withdrawal messages are dropped after debiting the sender.
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 0, vec![7, 8, 9])
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Multiple output messages should succeed");

    // Verify balance reduced by message values (10M + 5M + 0 = 15M)
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(85_000_000),
        "Balance should be reduced by total message value"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    assert_snark_received_message(&state, recipient1_id, 10_000_000);
    assert_snark_received_message(&state, recipient2_id, 5_000_000);
}

#[test]
fn test_snark_update_transfers_and_messages_combined() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    let message_recipient_id = test_account_id(302);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);
    create_snark_account_with_balance(&mut state, message_recipient_id, 0);

    // Create update with both transfers and messages using SnarkUpdateBuilder
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 25_000_000)
        .with_output_message(message_recipient_id, 10_000_000, vec![42, 43, 44])
        // Bridge-gateway non-withdrawal messages are dropped after debiting the sender.
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 5_000_000, vec![45, 46, 47])
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Combined transfers and messages should succeed");

    // Verify balances (100M - 25M - 15M = 60M)
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(60_000_000),
        "Sender should have 100M - 25M - 15M = 60M"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(25_000_000),
        "Recipient should receive 25M"
    );

    assert_snark_received_message(&state, message_recipient_id, 10_000_000);
}

fn assert_snark_received_message(
    state: &OLState,
    recipient_id: AccountId,
    expected_balance_sat: u64,
) {
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(state, recipient_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(expected_balance_sat),
        "message recipient balance should increase"
    );
    assert_eq!(
        snark_account_state.inbox_mmr().num_entries(),
        1,
        "message recipient inbox should receive one entry"
    );
    assert_eq!(
        snark_account_state.next_inbox_msg_idx(),
        0,
        "received messages should remain unprocessed"
    );
}
