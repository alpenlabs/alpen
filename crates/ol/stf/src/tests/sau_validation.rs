//! Tests for snark account update validation errors.

use strata_acct_types::{AcctError, BitcoinAmount, TxEffects};
use strata_ledger_types::{
    IAccountState, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor,
};
use strata_snark_acct_types::Seqno;

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
    let initial_sender_balance = BitcoinAmount::from_sat(100_000_000);
    let initial_recipient_balance = BitcoinAmount::zero();
    let initial_seqno = *lookup_snark_state(&state, snark_id).seqno().inner();

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

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        initial_sender_balance,
        "sender balance should not change after invalid sequence"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        initial_seqno,
        "sender seqno should not change after invalid sequence"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        initial_recipient_balance,
        "recipient balance should not change after invalid sequence"
    );
}

#[test]
fn test_snark_update_replay_across_blocks_fails() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    create_empty_account(&mut state, recipient_id);

    let transfer_amount = 10_000_000;
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, transfer_amount)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let block1 = execute_tx_in_block(&mut state, genesis_block.header(), tx.clone(), 1, 1)
        .expect("First SAU execution should succeed");

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "sender balance should reflect first execution"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "sender seqno should increment after first execution"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(transfer_amount),
        "recipient should receive the first execution"
    );

    let result = execute_tx_in_block(&mut state, block1.header(), tx, 2, 1);
    match result {
        Err(e) => match e.into_base() {
            ExecError::Acct(AcctError::InvalidUpdateSequence {
                account_id,
                expected,
                got,
            }) => {
                assert_eq!(account_id, snark_id);
                assert_eq!(expected, 1);
                assert_eq!(got, 0);
            }
            err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
        },
        Ok(_) => panic!("Replayed SAU should fail"),
    }

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "sender balance should not change after replay failure"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "sender seqno should not change after replay failure"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(transfer_amount),
        "recipient balance should not change after replay failure"
    );
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
    let initial_sender_balance = BitcoinAmount::from_sat(50_000_000);
    let initial_recipient_balance = BitcoinAmount::zero();
    let initial_seqno = *lookup_snark_state(&state, snark_id).seqno().inner();

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

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        initial_sender_balance,
        "sender balance should not change after insufficient balance"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        initial_seqno,
        "sender seqno should not change after insufficient balance"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        initial_recipient_balance,
        "recipient balance should not change after insufficient balance"
    );
}

#[test]
fn test_snark_update_nonexistent_recipient() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let nonexistent_id = test_account_id(TEST_NONEXISTENT_ID); // Not created

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    let initial_sender_balance = BitcoinAmount::from_sat(100_000_000);
    let initial_seqno = *lookup_snark_state(&state, snark_id).seqno().inner();

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

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        initial_sender_balance,
        "sender balance should not change after nonexistent recipient"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        initial_seqno,
        "sender seqno should not change after nonexistent recipient"
    );
    assert!(
        state.get_account_state(nonexistent_id).unwrap().is_none(),
        "failed transfer should not create the nonexistent recipient account"
    );
}

#[test]
fn test_snark_update_rejects_non_snark_target() {
    let mut state = create_test_genesis_state();
    let target_id = test_account_id(300);

    create_empty_account(&mut state, target_id);
    let genesis_block = setup_genesis_with_snark_accounts(&mut state, &[]);

    let initial_target_state = state.get_account_state(target_id).unwrap().unwrap().clone();

    let tx = create_unchecked_snark_update(
        target_id,
        0,
        get_test_state_root(2),
        0,
        TxEffects::default(),
    );

    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, 1, 1);

    match result {
        Err(e) => match e.into_base() {
            ExecError::IncorrectTxTargetType => {}
            err => panic!("Expected IncorrectTxTargetType, got: {err:?}"),
        },
        Ok(_) => panic!("SAU targeting a non-snark account should fail"),
    }

    let target_state = state.get_account_state(target_id).unwrap().unwrap();
    assert_eq!(
        target_state.balance(),
        initial_target_state.balance(),
        "non-snark target balance should not change after failed SAU"
    );
    assert!(
        target_state.as_snark_account().is_err(),
        "failed SAU should not convert the target into a snark account"
    );
}

#[test]
fn test_snark_update_rejects_max_sequence_number() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    create_empty_account(&mut state, recipient_id);

    let initial_state_root = lookup_snark_state(&state, snark_id).inner_state_root();
    state
        .update_account(snark_id, |ol_account_state| {
            ol_account_state
                .as_snark_account_mut()
                .expect("test account should be a snark account")
                .set_proof_state_directly(initial_state_root, 0, Seqno::from(u64::MAX));
        })
        .expect("test should update snark account sequence number");

    let initial_sender_balance = BitcoinAmount::from_sat(100_000_000);
    let initial_recipient_balance = BitcoinAmount::zero();
    let initial_seqno = *lookup_snark_state(&state, snark_id).seqno().inner();

    let mut effects = TxEffects::default();
    effects.push_transfer(recipient_id, 1);
    let tx = create_unchecked_snark_update(snark_id, u64::MAX, get_test_state_root(2), 0, effects);

    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, 1, 1);

    match result {
        Err(e) => match e.into_base() {
            ExecError::MaxSeqNumberReached { account_id } => {
                assert_eq!(account_id, snark_id);
            }
            err => panic!("Expected MaxSeqNumberReached, got: {err:?}"),
        },
        Ok(_) => panic!("SAU at max sequence number should fail"),
    }

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        initial_sender_balance,
        "sender balance should not change after max sequence failure"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        initial_seqno,
        "sender seqno should not change after max sequence failure"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        initial_recipient_balance,
        "recipient balance should not change after max sequence failure"
    );
}
