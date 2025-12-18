//! Tests for snark account operations including verification and state transitions

use strata_acct_types::{AccountId, AcctError, BitcoinAmount, Hash, MsgPayload};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_manifest_types::DepositIntentLogData;
use strata_identifiers::{AccountSerial, Buf32, L1BlockId, SubjectId, WtxidsRoot};
use strata_ledger_types::*;
use strata_msg_fmt::Msg;
use strata_ol_chain_types_new::{
    SimpleWithdrawalIntentLogData, SnarkAccountUpdateLogData, SnarkAccountUpdateTxPayload,
    TransactionPayload,
};
use strata_ol_msg_types::{WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, OutputMessage, OutputTransfer, ProofState, SnarkAccountUpdate,
    SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData, UpdateOutputs,
};

use crate::{
    assembly::BlockComponents,
    constants::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL},
    context::BlockInfo,
    errors::ExecError,
    test_utils::{execute_block, execute_block_with_outputs, test_account_id, test_l1_block_id},
    verification::*,
};

// === Test Helpers ===

/// Creates genesis block with accounts and deposits, returns (genesis_header, account_serials)
/// This properly handles the genesis terminal block with ASM manifest
fn setup_genesis_with_snark_account(
    state: &mut OLState,
    snark_id: AccountId,
    initial_deposit: u64,
) -> (strata_ol_chain_types_new::OLBlockHeader, AccountSerial) {
    // Create snark account
    let vk = PredicateKey::always_accept();
    let initial_state_root = Hash::from([1u8; 32]);
    let snark_state = OLSnarkAccountState::new_fresh(vk, initial_state_root);
    let new_acct_data = NewAccountData::new_empty(AccountTypeState::Snark(snark_state));
    let snark_serial = state
        .create_new_account(snark_id, new_acct_data)
        .expect("Should create snark account");

    // Create deposit manifest
    let dest_subject = SubjectId::from([42u8; 32]);
    let deposit_log_data = DepositIntentLogData::new(snark_serial, dest_subject, initial_deposit);
    let deposit_log_payload =
        strata_codec::encode_to_vec(&deposit_log_data).expect("Should encode deposit log data");

    let deposit_log = AsmLogEntry::from_msg(
        strata_asm_manifest_types::DEPOSIT_INTENT_ASM_LOG_TYPE_ID,
        deposit_log_payload,
    )
    .expect("Should create deposit log");

    let manifest = AsmManifest::new(
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![deposit_log],
    );

    // Execute genesis block (terminal with manifest)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_components = BlockComponents::new_manifests(vec![manifest]);
    let genesis_block = execute_block(state, &genesis_info, None, genesis_components)
        .expect("Genesis should execute");

    (genesis_block.header().clone(), snark_serial)
}

/// Helper to create additional empty accounts (for testing transfers/messages)
fn create_empty_account(state: &mut OLState, account_id: AccountId) -> AccountSerial {
    let empty_state = AccountTypeState::Empty;
    let new_acct_data = NewAccountData::new_empty(empty_state);
    state
        .create_new_account(account_id, new_acct_data)
        .expect("Should create empty account")
}

/// Helper to create a basic snark account update transaction
fn create_update_tx(
    target: AccountId,
    seq_no: u64,
    new_state_root: Hash,
    next_inbox_msg_idx: u64,
    outputs: UpdateOutputs,
) -> TransactionPayload {
    let new_proof_state = ProofState::new(new_state_root, next_inbox_msg_idx);
    let operation_data = UpdateOperationData::new(
        seq_no,
        new_proof_state,
        vec![],
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let sau_tx_payload = SnarkAccountUpdateTxPayload::new(target, update_container);
    TransactionPayload::SnarkAccountUpdate(sau_tx_payload)
}

/// Helper to execute a transaction in a non-genesis block
fn execute_tx_in_block(
    state: &mut OLState,
    parent_header: &strata_ol_chain_types_new::OLBlockHeader,
    tx: TransactionPayload,
) -> Result<(), ExecError> {
    let block_info = BlockInfo::new(1001000, 1, 1); // slot 1, epoch 1
    let components = BlockComponents::new_txs(vec![tx]);
    execute_block(state, &block_info, Some(parent_header), components).map(|_| ())
}

// === Tests ===

#[test]
fn test_snark_account_deposit_and_withdrawal() {
    // Start with empty genesis state
    let mut state = OLState::new_genesis();

    // Create a snark account in the state
    let snark_account_id = test_account_id(100);
    let initial_state_root = Hash::from([1u8; 32]);

    // Create a OLSnarkAccountState with always-accept predicate key for testing
    let vk = PredicateKey::always_accept();
    let snark_state = OLSnarkAccountState::new_fresh(vk, initial_state_root);

    let new_acct_data = NewAccountData::new_empty(AccountTypeState::Snark(snark_state));
    let snark_serial = state
        .create_new_account(snark_account_id, new_acct_data)
        .expect("Should create snark account");

    // Note: Bridge gateway is a special account and doesn't need to exist in the ledger

    // Create a genesis block with a manifest containing a deposit to the snark account
    let deposit_amount = 150_000_000u64; // 1.5 BTC in satoshis (must be enough to cover withdrawal)
    let dest_subject = SubjectId::from([42u8; 32]);

    // Create a deposit intent log in the manifest
    let deposit_log_data = DepositIntentLogData::new(snark_serial, dest_subject, deposit_amount);
    let deposit_log_payload =
        strata_codec::encode_to_vec(&deposit_log_data).expect("Should encode deposit log data");

    // Create an ASM log entry with the deposit intent
    let deposit_log = AsmLogEntry::from_msg(
        strata_asm_manifest_types::DEPOSIT_INTENT_ASM_LOG_TYPE_ID,
        deposit_log_payload,
    )
    .expect("Should create deposit log");

    // Create manifest with the deposit log
    let genesis_manifest = AsmManifest::new(
        0,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![deposit_log],
    );

    // Execute genesis block with the deposit manifest
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis_output =
        execute_block_with_outputs(&mut state, &genesis_info, None, genesis_components)
            .expect("Genesis block should execute");
    let genesis_block = genesis_output.completed_block();

    // Verify the deposit was processed
    let account_after_deposit = state
        .get_account_state(snark_account_id)
        .expect("Should get account state")
        .expect("Account should exist");
    assert_eq!(
        account_after_deposit.balance(),
        BitcoinAmount::from_sat(deposit_amount),
        "Account balance should reflect the deposit"
    );

    // Now create a snark account update transaction that produces a withdrawal
    let withdrawal_amount = 100_000_000u64; // Withdraw exactly 1 BTC (required denomination)
    let withdrawal_dest_desc = b"bc1qexample".to_vec(); // Example Bitcoin address descriptor
    let withdrawal_msg_data =
        WithdrawalMsgData::new(0, withdrawal_dest_desc.clone()).expect("Valid withdrawal data");

    // Encode the withdrawal message data using the msg-fmt library
    let encoded_withdrawal_body = strata_codec::encode_to_vec(&withdrawal_msg_data)
        .expect("Should encode withdrawal message");

    // Create OwnedMsg with proper format
    let withdrawal_msg =
        strata_msg_fmt::OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body)
            .expect("Should create withdrawal message");

    // Convert to bytes for the MsgPayload
    let withdrawal_payload_data = withdrawal_msg.to_vec();

    // Create the withdrawal message payload (sent to bridge gateway)
    let withdrawal_payload = MsgPayload::new(
        BitcoinAmount::from_sat(withdrawal_amount),
        withdrawal_payload_data,
    );

    // Create the output message to the bridge gateway account
    let bridge_gateway_id = BRIDGE_GATEWAY_ACCT_ID;
    let output_message = OutputMessage::new(bridge_gateway_id, withdrawal_payload);

    // Create the update outputs with the withdrawal message
    let update_outputs = UpdateOutputs::new(vec![], vec![output_message]);

    // Create the snark account update operation data
    let new_seqno = 0u64; // First sequence number (account expects seq_no=0)
    let new_state_root = Hash::from([2u8; 32]); // New state after update
    let new_proof_state = ProofState::new(new_state_root, 0);

    let operation_data = UpdateOperationData::new(
        new_seqno,
        new_proof_state.clone(),
        vec![],                  // No messages consumed
        LedgerRefs::new_empty(), // No ledger references
        update_outputs,
        vec![], // No extra data
    );

    // Create the snark account update
    let base_update = SnarkAccountUpdate::new(
        operation_data,
        vec![0u8; 32], // Dummy proof for testing
    );

    // Create accumulator proofs (empty for testing)
    let accumulator_proofs = UpdateAccumulatorProofs::new(
        vec![],                       // No inbox proofs
        LedgerRefProofs::new(vec![]), // No ledger ref proofs
    );

    // Create the update container
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);

    // Create the snark account update transaction
    let sau_tx_payload = SnarkAccountUpdateTxPayload::new(snark_account_id, update_container);
    let sau_tx = TransactionPayload::SnarkAccountUpdate(sau_tx_payload);

    // Create block 1 with the withdrawal transaction
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1_components = BlockComponents::new_txs(vec![sau_tx]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    let block1 = block1_output.completed_block();

    // Verify the withdrawal was processed
    let account_after_withdrawal = state
        .get_account_state(snark_account_id)
        .expect("Should get account state")
        .expect("Account should exist");

    // Balance should be reduced by withdrawal amount
    let expected_balance = deposit_amount - withdrawal_amount; // 150M - 100M = 50M satoshis
    assert_eq!(
        account_after_withdrawal.balance(),
        BitcoinAmount::from_sat(expected_balance),
        "Account balance should be reduced by withdrawal amount"
    );

    // Verify that logs were emitted
    let logs = block1_output.outputs().logs();
    let mut snark_update_found = false;
    let mut withdrawal_found = false;

    for log in logs {
        // Check if it's a snark account update log (from the snark account)
        if log.account_serial() == snark_serial
            && let Ok(update_log) =
                strata_codec::decode_buf_exact::<SnarkAccountUpdateLogData>(log.payload())
        {
            snark_update_found = true;
            // The update log indicates the snark account was updated
            assert_eq!(update_log.new_msg_idx(), 0, "Message index should be 0");
        }

        // Check if it's a withdrawal intent log (from the bridge gateway)
        if log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL
            && let Ok(withdrawal_log) =
                strata_codec::decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        {
            withdrawal_found = true;

            // Verify the withdrawal details
            assert_eq!(
                withdrawal_log.amt, withdrawal_amount,
                "Withdrawal amount should match"
            );

            assert_eq!(
                withdrawal_log.dest.as_slice(),
                withdrawal_dest_desc.as_slice(),
                "Withdrawal destination should match"
            );
        }
    }

    assert!(snark_update_found, "test: missing snark account log");
    assert!(withdrawal_found, "test: missing withdrawal intent log");
}

#[test]
fn test_snark_update_invalid_sequence_number() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to submit update with wrong sequence number (should be 1, but we use 5)
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );
    let bad_tx = create_update_tx(snark_id, 5, Hash::from([2u8; 32]), 0, outputs);

    // Execute and expect failure
    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(result.is_err(), "Update with wrong sequence should fail");
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. }) => {
            assert_eq!(expected, 1);
            assert_eq!(got, 5);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {:?}", err),
    }
}

#[test]
fn test_snark_update_insufficient_balance() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit of only 50M sats
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 50_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Try to send 100M sats (more than balance)
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(100_000_000),
        )],
        vec![],
    );
    let bad_tx = create_update_tx(snark_id, 1, Hash::from([2u8; 32]), 0, outputs);

    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

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
        err => panic!("Expected InsufficientBalance, got: {:?}", err),
    }
}

#[test]
fn test_snark_update_nonexistent_recipient() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let nonexistent_id = test_account_id(999); // Not created

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Try to send to non-existent account
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            nonexistent_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );
    let bad_tx = create_update_tx(snark_id, 1, Hash::from([2u8; 32]), 0, outputs);

    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(
        result.is_err(),
        "Update to non-existent account should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::NonExistentAccount(id)) => {
            assert_eq!(id, nonexistent_id);
        }
        err => panic!("Expected NonExistentAccount, got: {:?}", err),
    }
}

#[test]
fn test_snark_update_invalid_message_index() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create update claiming to have processed 5 messages (but inbox is empty)
    // next_inbox_msg_idx jumps from 0 to 5, but processed_messages is empty
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    let new_proof_state = ProofState::new(Hash::from([2u8; 32]), 5); // Claim we're at idx 5
    let operation_data = UpdateOperationData::new(
        1,
        new_proof_state,
        vec![], // But no messages processed!
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let bad_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
));
    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(
        result.is_err(),
        "Update with wrong message index should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(expected, 0); // Should stay at 0
            assert_eq!(got, 5); // But claimed 5
        }
        err => panic!("Expected InvalidMsgIndex, got: {:?}", err),
    }
}

#[test]
fn test_snark_update_success_with_transfer() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create valid update with transfer
    let transfer_amount = 30_000_000u64;
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(transfer_amount),
        )],
        vec![],
    );
    let tx = create_update_tx(snark_id, 1, Hash::from([2u8; 32]), 0, outputs);

    let result = execute_tx_in_block(&mut state, &genesis_header, tx);
    assert!(
        result.is_ok(),
        "Valid update should succeed: {:?}",
        result.err()
);
    // Verify balances
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(70_000_000),
        "Snark account balance should be 100M - 30M"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient should receive 30M"
    );
}
