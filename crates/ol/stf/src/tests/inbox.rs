//! Tests for inbox operations including message insertion, processing, and validation

use ssz_primitives::FixedBytes;
use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload, RawMerkleProof};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{GamTxPayload, TransactionPayload};
use strata_ol_state_types::OLState;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, MessageEntry, MessageEntryProof, OutputTransfer, ProofState,
    SnarkAccountUpdate, SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData,
    UpdateOutputs,
};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID,
    errors::ExecError,
    test_utils::{
        InboxMmrTracker, SnarkUpdateBuilder, create_empty_account, create_test_genesis_state,
        execute_tx_in_block, get_snark_state_expect, get_test_recipient_account_id,
        get_test_snark_account_id, get_test_state_root, setup_genesis_with_snark_account,
        test_account_id,
    },
};

#[test]
fn test_snark_inbox_message_insertion() {
    let mut state = create_test_genesis_state();
    let snark_id = test_account_id(100);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Send a message to snark account via GAM(Generic Account Message) tx (from sequencer,
    // value=0)
    let msg_data = vec![1u8, 2, 3, 4, 5];

    // Create GAM transaction
    let gam_tx_payload =
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload");
    let gam_tx = TransactionPayload::GenericAccountMessage(gam_tx_payload);

    // Execute transaction
    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch);
    assert!(
        result.is_ok(),
        "GAM transaction should succeed: {:?}",
        result.err()
    );

    // Verify the message was added to inbox
    let (snark_account, snark_state) = get_snark_state_expect(&state, snark_id);

    // Check that inbox MMR now has 1 entry (from GAM)
    assert_eq!(
        snark_state.inbox_mmr().num_entries(),
        1,
        "Inbox should have 1 message (GAM)"
    );

    // Check the seq no of the sender
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        0,
        "Sender account seq no should not increase for GAM"
    );

    // Balance unchanged (GAM messages have 0 value)
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Snark account balance should be unchanged"
    );
}

#[test]
fn test_snark_update_process_inbox_message_with_valid_mmr_proof() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create parallel MMR tracker to generate proofs
    let mut inbox_tracker = InboxMmrTracker::new();

    // Step 1: Send a message to snark account inbox
    let msg_data = vec![1u8, 2, 3, 4];
    let gam_tx = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
    );
    let (slot, epoch) = (1, 0);
    let blk1 = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch)
        .expect("GAM should succeed");
    let header = blk1.header();

    // Track the message in parallel MMR (must match exactly what was inserted)
    let gam_msg_entry = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        epoch, // epoch when message was added
        MsgPayload::new(BitcoinAmount::from_sat(0), msg_data),
    );

    let gam_proof = inbox_tracker.add_message(&gam_msg_entry);

    // Step 2: Verify the parallel MMR matches the actual inbox MMR
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_state = snark_account.as_snark_account().unwrap();
    let prev_seq_no = snark_state.seqno();

    assert_eq!(
        snark_state.inbox_mmr().num_entries(),
        inbox_tracker.num_entries(),
        "Parallel MMR must stay synchronized with actual inbox MMR"
    );
    assert_eq!(snark_state.inbox_mmr().num_entries(), 1);

    // The snark account starts with next_msg_read_idx = 0 (no messages processed yet)
    assert_eq!(snark_state.next_inbox_msg_idx(), 0);

    // Step 3: Create update that indicates that the GAM message was processed.
    // Just to have something in the outputs, include a transfer.
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    // After processing 1 message starting at index 0, next_msg_read_idx should be 1
    let new_proof_state = ProofState::new(get_test_state_root(2), 1);
    let operation_data = UpdateOperationData::new(
        0, // seq_no
        new_proof_state,
        vec![gam_msg_entry], // processed_messages
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(
        vec![gam_proof], // inbox_proofs
        LedgerRefProofs::new(vec![]),
    );
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let update_tx = TransactionPayload::SnarkAccountUpdate(
        strata_ol_chain_types_new::SnarkAccountUpdateTxPayload::new(snark_id, update_container),
    );

    // Step 4: Execute the update
    let (slot, epoch) = (2, 0);
    let result = execute_tx_in_block(&mut state, header, update_tx, slot, epoch);
    assert!(
        result.is_ok(),
        "Update with valid message proof should succeed: {:?}",
        result.err()
    );

    // Verify the update was applied
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(90_000_000),
        "Sender account should be debited"
    );

    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        prev_seq_no.inner() + 1,
        "Sender seq no should increment"
    );

    let snark_state = snark_account.as_snark_account().unwrap();
    assert_eq!(
        snark_state.next_inbox_msg_idx(),
        1,
        "Next inbox msg index should increment"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_invalid_message_index() {
    let mut state = create_test_genesis_state();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account with balance (no deposit message)
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    // Create proof state claiming to have processed 5 messages (but inbox is empty)
    let new_proof_state = ProofState::new(get_test_state_root(2), 5); // Claim we're at idx 5
    let operation_data = UpdateOperationData::new(
        0, // the first update, seq_no = 0
        new_proof_state,
        vec![], // No messages processed
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let invalid_tx = TransactionPayload::SnarkAccountUpdate(
        strata_ol_chain_types_new::SnarkAccountUpdateTxPayload::new(snark_id, update_container),
    );

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update with wrong message index should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(expected, 0); // Should stay at 0
            assert_eq!(got, 5); // But claimed 5
        }
        err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_invalid_message_proof() {
    let mut state = create_test_genesis_state();
    let snark_id = test_account_id(100);

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Step 1: Send a gam message to snark's inbox
    let msg_data = vec![1u8, 2, 3, 4];
    let gam_tx = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
    );
    let (slot, epoch) = (1, 0);
    let blk = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch)
        .expect("GAM should succeed");
    let header = blk.header();

    // Verify the message was added to inbox
    let (_, snark_state) = get_snark_state_expect(&state, snark_id);
    assert_eq!(
        snark_state.inbox_mmr().num_entries(),
        1,
        "1 inbox msg entry after gam message tx "
    );
    assert_eq!(
        snark_state.next_inbox_msg_idx(),
        0,
        "next to be processed msg idx should be 0"
    );

    // Step 2: Create update with INVALID proof for the gam message (index 0)
    // First create msg entry
    let deposit_msg = MessageEntry::new(
        BRIDGE_GATEWAY_ACCT_ID,
        0,
        MsgPayload::new(BitcoinAmount::from(0), msg_data),
    );

    // Create an invalid proof with bogus cohashes
    let invalid_raw_proof = RawMerkleProof {
        cohashes: vec![FixedBytes::<32>::from([0xff; 32])].into(),
    };
    let invalid_msg_proof = MessageEntryProof::new(deposit_msg.clone(), invalid_raw_proof);

    // Create update
    let outputs = UpdateOutputs::new(vec![], vec![]);

    let new_msg_idx = 1;
    let new_proof_state = ProofState::new(get_test_state_root(2), new_msg_idx);
    let operation_data = UpdateOperationData::new(
        0,
        new_proof_state,
        vec![deposit_msg],
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs =
        UpdateAccumulatorProofs::new(vec![invalid_msg_proof], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let invalid_tx = TransactionPayload::SnarkAccountUpdate(
        strata_ol_chain_types_new::SnarkAccountUpdateTxPayload::new(snark_id, update_container),
    );

    // Step 3: Execute and expect failure
    let (slot, epoch) = (2, 0);
    let result = execute_tx_in_block(&mut state, header, invalid_tx, slot, epoch);

    assert!(
        result.is_err(),
        "Update with invalid message proof should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
            assert_eq!(msg_idx, 0, "Should fail on message index 0");
        }
        err => panic!("Expected InvalidMessageProof, got: {err:?}"),
    }
}

#[test]
fn test_snark_update_skip_message_out_of_order() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Step 1: Send TWO messages to inbox
    let msg1_data = vec![1u8, 2, 3, 4];
    let gam_tx1 = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg1_data.clone()).expect("Should create GAM payload"),
    );
    let (slot, epoch) = (1, 0);
    let blk = execute_tx_in_block(
        &mut state,
        genesis_block.header(),
        gam_tx1.clone(),
        slot,
        epoch,
    )
    .expect("GAM 1 should succeed");
    let header = blk.header();

    let msg2_data = vec![5u8, 6, 7, 8];
    let gam_tx2 = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg2_data.clone()).expect("Should create GAM payload"),
    );
    let blk = execute_tx_in_block(&mut state, header, gam_tx2, slot + 1, epoch)
        .expect("GAM 2 should succeed");
    let header = blk.header();

    // Verify we have 2 messages (2 GAMs, no deposit)
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_state = snark_account.as_snark_account().unwrap();
    assert_eq!(snark_state.inbox_mmr().num_entries(), 2);

    // Step 2: Try to process only the SECOND message (skipping first)
    // This should fail because messages must be processed in order starting from index 0
    let msg2_entry = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        0,
        MsgPayload::new(BitcoinAmount::from_sat(0), msg2_data),
    );

    // The proof would be for index 1, but we're at index 0
    // This will fail the message index check, not the proof check
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    // Claiming to process 1 message but jumping to index 2 (skipping first GAM)
    let new_proof_state = ProofState::new(get_test_state_root(2), 2); // Skip to index 2
    let operation_data = UpdateOperationData::new(
        0,
        new_proof_state,
        vec![msg2_entry], // Only 1 message processed
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let invalid_tx = TransactionPayload::SnarkAccountUpdate(
        strata_ol_chain_types_new::SnarkAccountUpdateTxPayload::new(snark_id, update_container),
    );

    // Step 3: Execute and expect failure
    let (slot, epoch) = (3, 0);
    let result = execute_tx_in_block(&mut state, header, invalid_tx, slot, epoch);

    assert!(result.is_err(), "Update skipping messages should fail");
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(
                expected, 1,
                "Should expect index 1 (current 0 + 1 message processed)"
            );
            assert_eq!(got, 2, "But got index 2 (skipped from 0 to 2)");
        }
        err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
    }
}
