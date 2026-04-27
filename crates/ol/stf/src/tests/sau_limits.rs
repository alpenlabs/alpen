//! Tests for snark account u64 and Bitcoin-supply boundaries.

use strata_acct_types::{BitcoinAmount, TxEffects};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};

use crate::{errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_max_bitcoin_supply() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account with maximum Bitcoin supply
    // Bitcoin max supply is 21M BTC = 2.1 quadrillion satoshis
    let max_bitcoin_sats = 2_100_000_000_000_000u64; // 21M BTC in sats
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, max_bitcoin_sats);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try multiple transfers that would exceed total Bitcoin supply
    let mut effects = TxEffects::default();
    effects.push_transfer(recipient_id, max_bitcoin_sats);
    effects.push_transfer(recipient_id, 1); // Even 1 sat more exceeds balance

    let invalid_tx = create_unchecked_snark_update(
        snark_id,
        0, // seq_no
        get_test_state_root(2),
        0, // new_msg_idx
        effects,
    );

    let (slot, epoch) = (1, 1);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    // Should fail due to insufficient balance
    assert!(result.is_err(), "Update exceeding balance should fail");

    match result.unwrap_err().into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    // Verify no state change
    let (ol_account_state, _) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(max_bitcoin_sats),
        "Balance should be unchanged after failed update"
    );
}

#[test]
fn test_snark_update_transfer_above_bitcoin_supply_accepted() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account that has balance exceeding 21M BTC
    // Bitcoin max supply is 21M BTC = 2.1 quadrillion satoshis
    // We'll give the account u64::MAX to test transfers larger than Bitcoin's supply
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, u64::MAX);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to transfer more than 21M BTC in a single transfer
    let transfer_amount = 2_100_000_000_000_001u64; // 21M BTC + 1 satoshi
    let expected_sender_balance = u64::MAX - transfer_amount;

    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient_id, transfer_amount)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    // This should succeed as the account has sufficient balance
    // The protocol doesn't enforce Bitcoin's 21M limit on individual transfers
    execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch)
        .expect("Transfer exceeding Bitcoin max supply should succeed if balance is available");

    // Verify the transfer was applied correctly
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(expected_sender_balance),
        "Sender balance should be reduced by transfer amount"
    );
    assert_eq!(
        *snark_account_state.seqno().inner(),
        1,
        "Sequence number should increment"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(transfer_amount),
        "Recipient should receive the transfer amount exceeding 21M BTC"
    );
}

#[test]
fn test_snark_update_overflow_u64_boundary() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient1_id = test_account_id(TEST_RECIPIENT_ID + 1); // Different test IDs
    let recipient2_id = test_account_id(TEST_RECIPIENT_ID + 2);

    // Setup: genesis with snark account with balance near u64::MAX
    let initial_balance = u64::MAX - 100; // Just under u64::MAX
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, initial_balance);

    // Create recipient accounts using helper
    create_empty_account(&mut state, recipient1_id);
    create_empty_account(&mut state, recipient2_id);

    // Test case 1: Try transfers that sum to more than available balance
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient1_id, u64::MAX - 100) // Max we can afford
        .with_transfer(recipient2_id, 101) // This exceeds balance
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 1);
    let result1 = execute_tx_in_block(&mut state, genesis_block.header(), tx1, slot, epoch);

    // Should fail due to insufficient balance
    match result1 {
        Err(e) => assert!(
            matches!(e.into_base(), ExecError::AmountOverflow),
            "Expected AmountOverflow"
        ),
        Ok(_) => panic!("Update with total exceeding available balance should fail"),
    }

    // Verify no state change occurred
    let (ol_account_state, _) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(initial_balance),
        "Balance should be unchanged after failed update"
    );

    // Test case 2: Try transfers where one is u64::MAX and another is 1
    // This tests overflow handling when summing transfers
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx2 = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_transfer(recipient1_id, u64::MAX) // Maximum u64 value
        .with_transfer(recipient2_id, 1) // Even 1 more would overflow
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let result2 = execute_tx_in_block(&mut state, genesis_block.header(), tx2, slot, epoch);

    match result2 {
        Err(e) => assert!(
            matches!(e.into_base(), ExecError::AmountOverflow),
            "Expected AmountOverflow"
        ),
        Ok(_) => panic!("Sending more than bitcoin limits should fail"),
    }

    // Verify recipients have no balance (no partial execution)
    let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient1.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient1 should have no balance after failed update"
    );

    let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
    assert_eq!(
        recipient2.balance(),
        BitcoinAmount::from_sat(0),
        "Recipient2 should have no balance after failed update"
    );

    // Test case 3: Verify that u64::MAX transfer works when balance is sufficient
    // Give snark account exactly u64::MAX balance
    let mut state3 = create_test_genesis_state();
    let genesis_block3 = setup_genesis_with_snark_account(&mut state3, snark_id, u64::MAX);
    create_empty_account(&mut state3, recipient1_id);

    let snark_account_state3 = lookup_snark_state(&state3, snark_id);
    let tx3 = SnarkUpdateBuilder::from_snark_state(snark_account_state3.clone())
        .with_transfer(recipient1_id, u64::MAX) // Transfer entire u64::MAX
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    execute_tx_in_block(&mut state3, genesis_block3.header(), tx3, slot, epoch)
        .expect("Transfer of u64::MAX should succeed when balance is sufficient");

    // Verify the transfer completed
    let (ol_account_state3, _) = lookup_snark_account_states(&state3, snark_id);
    assert_eq!(
        ol_account_state3.balance(),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance after transferring u64::MAX"
    );

    let recipient3 = state3.get_account_state(recipient1_id).unwrap().unwrap();
    assert_eq!(
        recipient3.balance(),
        BitcoinAmount::from_sat(u64::MAX),
        "Recipient should receive u64::MAX"
    );
}
