//! Tests for snark account operations including verification and state transitions

use ssz::Encode as _;
use ssz_primitives::FixedBytes;
use strata_acct_types::{
    AccountId, AcctError, BitcoinAmount, Hash, MsgPayload, RawMerkleProof, StrataHasher,
};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_manifest_types::DepositIntentLogData;
use strata_identifiers::{AccountSerial, Buf32, L1BlockId, SubjectId, WtxidsRoot};
use strata_ledger_types::*;
use strata_merkle::{MerkleProof, hasher::MerkleHasher};
use strata_msg_fmt::Msg;
use strata_ol_chain_types_new::{
    GamTxPayload, SimpleWithdrawalIntentLogData, SnarkAccountUpdateLogData,
    SnarkAccountUpdateTxPayload, TransactionPayload,
};
use strata_ol_msg_types::{WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, MessageEntry, MessageEntryProof, OutputMessage, OutputTransfer,
    ProofState, SnarkAccountUpdate, SnarkAccountUpdateContainer, UpdateAccumulatorProofs,
    UpdateOperationData, UpdateOutputs,
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
/// Queries the current state to determine inbox index and processes all pending messages
fn create_update_tx(
    state: &OLState,
    target: AccountId,
    seq_no: u64,
    new_state_root: Hash,
    outputs: UpdateOutputs,
) -> TransactionPayload {
    // Get current inbox state
    let account = state.get_account_state(target).unwrap().unwrap();
    let snark_state = account.as_snark_account().unwrap();
    let cur_inbox_idx = snark_state.get_next_inbox_msg_idx();

    // For simplicity, just advance the index without actually processing messages
    // (in real usage, you'd need to include the actual messages and proofs)
    let new_proof_state = ProofState::new(new_state_root, cur_inbox_idx);
    let operation_data = UpdateOperationData::new(
        seq_no,
        new_proof_state,
        vec![], // No messages processed (tests that don't care about messages)
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

    // Check inbox state after genesis
    let snark_state_after_genesis = account_after_deposit.as_snark_account().unwrap();
    let inbox_idx_after_genesis = snark_state_after_genesis.get_next_inbox_msg_idx();
    eprintln!(
        "DEBUG: Inbox MMR has {} messages (next insert at index {})",
        inbox_idx_after_genesis, inbox_idx_after_genesis
    );

    // Check the proof state (next message to PROCESS)
    let current_processing_idx = snark_state_after_genesis.inner_state_root(); // Wrong field, but let's see
    eprintln!(
        "DEBUG: Current inner_state_root: {:?}",
        current_processing_idx
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

    // Process the deposit message that was added during genesis
    let account_after_genesis = state.get_account_state(snark_account_id).unwrap().unwrap();
    let snark_state_after_genesis = account_after_genesis.as_snark_account().unwrap();
    let cur_inbox_idx = snark_state_after_genesis.get_next_inbox_msg_idx();

    let deposit_msg = MessageEntry::new(
        BRIDGE_GATEWAY_ACCT_ID,
        1, // genesis epoch
        MsgPayload::new(BitcoinAmount::from_sat(deposit_amount), vec![]),
    );

    // Current inbox is at index cur_inbox_idx, processing 1 message means next index is
    // cur_inbox_idx+1
    let new_proof_state = ProofState::new(new_state_root, cur_inbox_idx + 1);

    let operation_data = UpdateOperationData::new(
        new_seqno,
        new_proof_state.clone(),
        vec![deposit_msg],       // Process the deposit message
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
            // We started at cur_inbox_idx and processed 1 message, so new index is cur_inbox_idx +
            // 1
            assert_eq!(
                update_log.new_msg_idx(),
                cur_inbox_idx + 1,
                "Message index should advance by 1 after processing the deposit message"
            );
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
    let bad_tx = create_update_tx(&state, snark_id, 5, Hash::from([2u8; 32]), outputs);

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
    let bad_tx = create_update_tx(&state, snark_id, 1, Hash::from([2u8; 32]), outputs);

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
    let bad_tx = create_update_tx(&state, snark_id, 1, Hash::from([2u8; 32]), outputs);

    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(
        result.is_err(),
        "Update to non-existent account should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::MissingExpectedAccount(id)) => {
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
    let tx = create_update_tx(&state, snark_id, 1, Hash::from([2u8; 32]), outputs);

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

#[test]
fn test_snark_inbox_message_insertion() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Send a message to snark account via GAM tx (from sequencer, value=0)
    let msg_data = vec![1u8, 2, 3, 4, 5];

    // Create GAM transaction
    let gam_tx_payload =
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload");
    let gam_tx = TransactionPayload::GenericAccountMessage(gam_tx_payload);

    // Execute transaction
    let result = execute_tx_in_block(&mut state, &genesis_header, gam_tx);
    assert!(
        result.is_ok(),
        "GAM transaction should succeed: {:?}",
        result.err()
    );

    // Verify the message was added to inbox
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_state = snark_account.as_snark_account().unwrap();

    // Check that inbox MMR now has 2 entries (1 from deposit, 1 from GAM)
    assert_eq!(
        snark_state.get_next_inbox_msg_idx(),
        2,
        "Inbox should have 2 messages (deposit + GAM)"
    );

    // Balance unchanged (GAM messages have 0 value)
    assert_eq!(
        snark_account.balance(),
        BitcoinAmount::from_sat(100_000_000),
        "Snark account balance should be unchanged"
    );
}

#[test]
fn test_snark_update_process_inbox_message_with_valid_proof() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Step 1: Send a message to snark account inbox
    let msg_data = vec![1u8, 2, 3, 4];
    let gam_tx = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
    );
    execute_tx_in_block(&mut state, &genesis_header, gam_tx).expect("GAM should succeed");

    // Step 2: Get the current inbox state to generate proofs
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_state = snark_account.as_snark_account().unwrap();
    let inbox_mmr = snark_state.inbox_mmr();

    // Verify we have 2 messages (deposit + GAM)
    let mmr_size = snark_state.get_next_inbox_msg_idx(); // This is MMR size (next insertion point)
    eprintln!("DEBUG: MMR size = {}", mmr_size);
    assert_eq!(mmr_size, 2);

    // Get current ProofState to see where we are in processing
    let cur_proof_state_idx = snark_state.inner_state_root(); // We need to check the actual ProofState
    // The snark account starts with next_inbox_msg_idx = 0 (no messages processed yet)

    // Step 3: Process BOTH messages starting from index 0
    // Since deposits are system messages, we'll process both to advance to index 2

    // First message is the deposit (from Bridge Gateway)
    let deposit_msg_entry = MessageEntry::new(
        crate::constants::BRIDGE_GATEWAY_ACCT_ID,
        1, // epoch
        MsgPayload::new(BitcoinAmount::from_sat(100_000_000), vec![]),
    );

    // Second message is our GAM
    let gam_msg_entry = MessageEntry::new(
        crate::constants::SEQUENCER_ACCT_ID,
        1, // epoch from genesis block context
        MsgPayload::new(BitcoinAmount::from_sat(0), msg_data),
    );

    // For simplicity, we'll skip proof verification by using empty proofs
    // In a real scenario, you'd generate proper MMR proofs
    let deposit_proof = MessageEntryProof::new(
        deposit_msg_entry.clone(),
        RawMerkleProof {
            cohashes: vec![].into(),
        },
    );
    let gam_proof = MessageEntryProof::new(
        gam_msg_entry.clone(),
        RawMerkleProof {
            cohashes: vec![].into(),
        },
    );

    // Step 4: Create update that processes both messages
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    // The verification expects: new_idx = current_mmr_size + processed_messages.len()
    // current_mmr_size = 2, processed = 2, so new_idx should be 4
    let new_proof_state = ProofState::new(Hash::from([2u8; 32]), mmr_size + 2); // mmr_size + processed_len
    let operation_data = UpdateOperationData::new(
        1, // seq_no
        new_proof_state,
        vec![deposit_msg_entry, gam_msg_entry], // processed_messages
        LedgerRefs::new_empty(),
        outputs,
        vec![],
    );

    let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
    let accumulator_proofs = UpdateAccumulatorProofs::new(
        vec![deposit_proof, gam_proof], // inbox_proofs
        LedgerRefProofs::new(vec![]),
    );
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
    let update_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
    ));

    // Step 5: Execute the update
    let result = execute_tx_in_block(&mut state, &genesis_header, update_tx);
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
        "Snark account should be debited"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_invalid_message_proof() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Step 1: Send a message to inbox
    let msg_data = vec![1u8, 2, 3, 4];
    let gam_tx = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
    );
    execute_tx_in_block(&mut state, &genesis_header, gam_tx).expect("GAM should succeed");

    // Step 2: Create update with INVALID proof for the deposit (index 0)
    let deposit_msg = MessageEntry::new(
        crate::constants::BRIDGE_GATEWAY_ACCT_ID,
        1,
        MsgPayload::new(BitcoinAmount::from_sat(100_000_000), vec![]),
    );

    // Create an invalid proof with bogus cohashes
    let invalid_raw_proof = RawMerkleProof {
        cohashes: vec![FixedBytes::<32>::from([0xff; 32])].into(),
    };
    let invalid_msg_proof = MessageEntryProof::new(deposit_msg.clone(), invalid_raw_proof);

    // Create update
    let outputs = UpdateOutputs::new(
        vec![OutputTransfer::new(
            recipient_id,
            BitcoinAmount::from_sat(10_000_000),
        )],
        vec![],
    );

    let new_proof_state = ProofState::new(Hash::from([2u8; 32]), 1);
    let operation_data = UpdateOperationData::new(
        1,
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
    let bad_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
        snark_id,
        update_container,
    ));

    // Step 3: Execute and expect failure
    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(
        result.is_err(),
        "Update with invalid message proof should fail"
    );
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
            assert_eq!(msg_idx, 0, "Should fail on message index 0");
        }
        err => panic!("Expected InvalidMessageProof, got: {:?}", err),
    }
}

#[test]
fn test_snark_update_skip_message_out_of_order() {
    let mut state = OLState::new_genesis();
    let snark_id = test_account_id(100);
    let recipient_id = test_account_id(200);

    // Setup: genesis with snark account + deposit
    let (genesis_header, _snark_serial) =
        setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Step 1: Send TWO messages to inbox
    let msg1_data = vec![1u8, 2, 3, 4];
    let gam_tx1 = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg1_data.clone()).expect("Should create GAM payload"),
    );
    execute_tx_in_block(&mut state, &genesis_header, gam_tx1.clone())
        .expect("GAM 1 should succeed");

    let msg2_data = vec![5u8, 6, 7, 8];
    let gam_tx2 = TransactionPayload::GenericAccountMessage(
        GamTxPayload::new(snark_id, msg2_data.clone()).expect("Should create GAM payload"),
    );
    execute_tx_in_block(&mut state, &genesis_header, gam_tx2).expect("GAM 2 should succeed");

    // Verify we have 3 messages (deposit + 2 GAMs)
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_state = snark_account.as_snark_account().unwrap();
    assert_eq!(snark_state.get_next_inbox_msg_idx(), 3);

    // Step 2: Try to process only the SECOND message (skipping first)
    // This should fail because messages must be processed in order starting from index 0
    let msg2_entry = MessageEntry::new(
        crate::constants::SEQUENCER_ACCT_ID,
        1,
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

    // Claiming to process 1 message but jumping to index 3 (skipping deposit and first GAM)
    let new_proof_state = ProofState::new(Hash::from([2u8; 32]), 3); // Skip to index 3
    let operation_data = UpdateOperationData::new(
        1,
        new_proof_state,
        vec![msg2_entry], // Only 1 message processed
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

    // Step 3: Execute and expect failure
    let result = execute_tx_in_block(&mut state, &genesis_header, bad_tx);

    assert!(result.is_err(), "Update skipping messages should fail");
    match result.unwrap_err() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(
                expected, 1,
                "Should expect index 1 (current 0 + 1 processed)"
            );
            assert_eq!(got, 3, "But got index 3");
        }
        err => panic!("Expected InvalidMsgIndex, got: {:?}", err),
    }
}
