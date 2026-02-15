//! Tests for edge cases in value transfers

use strata_acct_types::{AccountId, AcctError, BitcoinAmount};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{SnarkAccountUpdateTxPayload, TransactionPayload};
use strata_ol_state_types::OLState;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, OutputTransfer, ProofState, SnarkAccountUpdate,
    SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData, UpdateOutputs,
};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID,
    errors::ExecError,
    test_utils::{
        SnarkUpdateBuilder, TEST_RECIPIENT_ID, create_empty_account, create_test_genesis_state,
        execute_tx_in_block, get_test_proof, get_test_recipient_account_id,
        get_test_snark_account_id, get_test_state_root, setup_genesis_with_snark_account,
        test_account_id,
    },
};

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

    let (slot, epoch) = (1, 0);
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

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx_nonzero, slot, epoch);

    // Should fail due to insufficient balance
    assert!(result.is_err(), "Transfer from zero balance should fail");

    match result.unwrap_err() {
        ExecError::Acct(AcctError::InsufficientBalance {
            requested,
            available,
        }) => {
            assert_eq!(requested, BitcoinAmount::from_sat(1));
            assert_eq!(available, BitcoinAmount::from_sat(0));
        }
        err => panic!("Expected InsufficientBalance, got: {err:?}"),
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

    let (slot, epoch) = (1, 0);
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

    let (slot, epoch) = (1, 0);
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
    let transfer1 = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(max_bitcoin_sats));
    let transfer2 = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(1)); // Even 1 sat more exceeds balance

    let update_outputs = UpdateOutputs::new(vec![transfer1, transfer2], vec![]);

    let seq_no = 0u64;
    let new_proof_state = ProofState::new(get_test_state_root(2), 0);
    let operation_data = UpdateOperationData::new(
        seq_no,
        new_proof_state,
        vec![],
        LedgerRefs::new_empty(),
        update_outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
    ));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    // Should fail due to insufficient balance
    assert!(result.is_err(), "Update exceeding balance should fail");

    match result.unwrap_err() {
        ExecError::Acct(AcctError::InsufficientBalance {
            requested,
            available,
        }) => {
            assert_eq!(requested, BitcoinAmount::from_sat(max_bitcoin_sats + 1));
            assert_eq!(available, BitcoinAmount::from_sat(max_bitcoin_sats));
        }
        err => panic!("Expected InsufficientBalance, got: {err:?}"),
    }

    // Verify no state change
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(max_bitcoin_sats),
        "Balance should be unchanged after failed update"
    );
}

#[test]
fn test_snark_update_single_transfer_exceeding_max_bitcoin_suceeds() {
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
    let more_than_max_bitcoin = 2_100_000_000_000_001u64; // 21M BTC + 1 satoshi

    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, more_than_max_bitcoin)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    // This should succeed as the account has sufficient balance
    // The protocol doesn't enforce Bitcoin's 21M limit on individual transfers
    assert!(
        result.is_ok(),
        "Transfer exceeding Bitcoin max supply should succeed if balance is available: {:?}",
        result.err()
    );

    // Verify the transfer was applied correctly
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(u64::MAX - more_than_max_bitcoin),
        "Sender balance should be reduced by transfer amount"
    );
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        1,
        "Sequence number should increment"
    );

    let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient.balance(),
        BitcoinAmount::from_sat(more_than_max_bitcoin),
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
    let tx1 = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient1_id, u64::MAX - 100) // Max we can afford
    .with_transfer(recipient2_id, 101) // This exceeds balance
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result1 = execute_tx_in_block(&mut state, genesis_block.header(), tx1, slot, epoch);

    // Should fail due to insufficient balance
    assert!(
        result1.is_err(),
        "Update with total exceeding available balance should fail"
    );

    assert!(
        matches!(
            result1,
            Err(ExecError::Acct(AcctError::BitcoinAmountOverflow))
        ),
        "Sending more than bitcoin limits"
    );

    // Verify no state change occurred
    let acct_state = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        acct_state.balance(),
        BitcoinAmount::from_sat(initial_balance),
        "Balance should be unchanged after failed update"
    );

    // Test case 2: Try transfers where one is u64::MAX and another is 1
    // This tests overflow handling when summing transfers
    let tx2 = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient1_id, u64::MAX) // Maximum u64 value
    .with_transfer(recipient2_id, 1) // Even 1 more would overflow
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let result2 = execute_tx_in_block(&mut state, genesis_block.header(), tx2, slot, epoch);

    assert!(
        matches!(
            result2,
            Err(ExecError::Acct(AcctError::BitcoinAmountOverflow))
        ),
        "Sending more than bitcoin limits"
    );

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

    let tx3 = SnarkUpdateBuilder::from_snark_state(
        state3
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient1_id, u64::MAX) // Transfer entire u64::MAX
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let result3 = execute_tx_in_block(&mut state3, genesis_block3.header(), tx3, slot, epoch);

    assert!(
        result3.is_ok(),
        "Transfer of u64::MAX should succeed when balance is sufficient: {:?}",
        result3.err()
    );

    // Verify the transfer completed
    let snark_account3 = state3.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account3.balance(),
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
