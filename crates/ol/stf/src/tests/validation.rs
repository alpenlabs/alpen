//! Tests for basic validation errors like sequence numbers, balance checks, and recipient
//! validation

use strata_acct_types::{AccountId, AcctError, BitcoinAmount};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_state_types::OLState;

use crate::{
    errors::ExecError,
    test_utils::{
        SnarkUpdateBuilder, TEST_NONEXISTENT_ID, create_empty_account,
        create_unchecked_snark_update, execute_tx_in_block, get_test_proof,
        get_test_recipient_account_id, get_test_snark_account_id, get_test_state_root,
        setup_genesis_with_snark_account, test_account_id,
    },
};

#[test]
fn test_snark_update_invalid_sequence_number() {
    let mut state = OLState::new_genesis();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to submit update with wrong sequence number (should be 0, but we use 5)
    let transfer = strata_snark_acct_types::OutputTransfer::new(
        recipient_id,
        BitcoinAmount::from_sat(10_000_000),
    );
    let outputs = strata_snark_acct_types::UpdateOutputs::new(vec![transfer], vec![]);
    let invalid_tx = create_unchecked_snark_update(
        snark_id,
        5, // wrong seq_no (should be 0)
        get_test_state_root(2),
        0, // new_msg_idx
        outputs,
    );

    // Execute and expect failure
    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(result.is_err(), "Update with wrong sequence should fail");
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. }) => {
            assert_eq!(expected, 0);
            assert_eq!(got, 5);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_insufficient_balance() {
    let mut state = OLState::new_genesis();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account of only 50M sats
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 50_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to send 100M sats (more than balance)
    let invalid_tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, 100_000_000)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update with insufficient balance should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InsufficientBalance {
            requested,
            available,
        }) => {
            assert_eq!(requested, BitcoinAmount::from_sat(100_000_000));
            assert_eq!(available, BitcoinAmount::from_sat(50_000_000));
        }
        err => panic!("Expected InsufficientBalance, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_nonexistent_recipient() {
    let mut state = OLState::new_genesis();
    let snark_id = get_test_snark_account_id();
    let nonexistent_id = test_account_id(TEST_NONEXISTENT_ID); // Not created

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Try to send to non-existent account
    let invalid_tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(nonexistent_id, 10_000_000)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update to non-existent account should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::MissingExpectedAccount(id)) => {
            assert_eq!(id, nonexistent_id);
        }
        err => panic!("Expected NonExistentAccount, got: {err:?}"),
    }
}
