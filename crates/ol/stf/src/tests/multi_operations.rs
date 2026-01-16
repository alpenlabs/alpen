//! Tests for multiple operations in a single update

use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload};
use strata_ledger_types::{IAccountState, IStateAccessor};
use strata_ol_chain_types_new::{SnarkAccountUpdateTxPayload, TransactionPayload};
use strata_ol_state_types::OLState;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, OutputMessage, OutputTransfer, ProofState, SnarkAccountUpdate,
    SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData, UpdateOutputs,
};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID,
    errors::ExecError,
    test_utils::{
        SnarkUpdateBuilder, create_empty_account, execute_tx_in_block, get_test_proof,
        get_test_recipient_account_id, get_test_snark_account_id, get_test_state_root,
        setup_genesis_with_snark_account, test_account_id,
    },
};

#[test]
fn test_snark_update_multiple_transfers() {
    let mut state = OLState::new_genesis();
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

    let (slot, epoch) = (1, 0);
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
    let mut state = OLState::new_genesis();
    let snark_id = get_test_snark_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create multiple output messages
    let msg1_payload = MsgPayload::new(BitcoinAmount::from_sat(10_000_000), vec![1, 2, 3]);
    let msg2_payload = MsgPayload::new(BitcoinAmount::from_sat(5_000_000), vec![4, 5, 6]);
    let msg3_payload = MsgPayload::new(BitcoinAmount::from_sat(0), vec![7, 8, 9]);

    let output_message1 = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg1_payload);
    let output_message2 = OutputMessage::new(SEQUENCER_ACCT_ID, msg2_payload);
    let output_message3 = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg3_payload);

    // Create update with multiple messages
    let update_outputs = UpdateOutputs::new(
        vec![],
        vec![output_message1, output_message2, output_message3],
    );

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
    let mut state = OLState::new_genesis();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create update with both transfers and messages
    let transfer = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(25_000_000));
    let msg_payload = MsgPayload::new(BitcoinAmount::from_sat(15_000_000), vec![42, 43, 44]);
    let output_message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg_payload);

    let update_outputs = UpdateOutputs::new(vec![transfer], vec![output_message]);

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
    let mut state = OLState::new_genesis();
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

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

    assert!(result.is_err(), "Update exceeding balance should fail");
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InsufficientBalance {
            requested,
            available,
        }) => {
            assert_eq!(requested, BitcoinAmount::from_sat(110_000_000));
            assert_eq!(available, BitcoinAmount::from_sat(100_000_000));
        }
        err => panic!("Expected InsufficientBalance, got: {err:?}"),
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
