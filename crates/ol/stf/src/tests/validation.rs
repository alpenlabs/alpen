//! Tests for basic validation errors like sequence numbers, balance checks, and recipient
//! validation

use strata_acct_types::{AcctError, TxEffects};

use crate::{errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_invalid_sequence_number() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to submit update with wrong sequence number (should be 0, but we use 5)
    let mut effects = TxEffects::default();
    effects.push_transfer(recipient_id, 10_000_000);
    let invalid_tx = create_unchecked_snark_update(
        snark_id,
        5, // wrong seq_no (should be 0)
        get_test_state_root(2),
        0, // new_msg_idx
        effects,
    );

    // Execute and expect failure
    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(result.is_err(), "Update with wrong sequence should fail");
    match result.unwrap_err().into_base() {
        ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. }) => {
            assert_eq!(expected, 0);
            assert_eq!(got, 5);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_insufficient_balance() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account of only 50M sats
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 50_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to send 100M sats (more than balance)
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let invalid_tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, 100_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update with insufficient balance should fail"
    );
    match result.unwrap_err().into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_nonexistent_recipient() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let nonexistent_id = test_account_id(TEST_NONEXISTENT_ID); // Not created

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Try to send to non-existent account
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let invalid_tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(nonexistent_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update to non-existent account should fail"
    );
    match result.unwrap_err().into_base() {
        ExecError::UnknownAccount(id) => {
            assert_eq!(id, nonexistent_id);
        }
        err => panic!("Expected UnknownAccount, got: {err:?}"),
    }
}
