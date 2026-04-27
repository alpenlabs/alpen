//! Deposit-withdraw tests for end-to-end workflows

use strata_acct_types::{BitcoinAmount, Hash, MessageEntry, MsgPayload};
use strata_asm_common::{AsmLogEntry, AsmManifest, logging::debug};
use strata_asm_logs::DepositLog;
use strata_identifiers::{Buf32, SubjectId, SubjectIdBytes, WtxidsRoot};
use strata_ledger_types::*;
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::{OLBlockHeader, OLTransaction};
use strata_ol_msg_types::{
    DEFAULT_OPERATOR_FEE, DEPOSIT_MSG_TYPE_ID, WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData,
};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL,
    assembly::{BlockComponents, ConstructBlockOutput},
    context::BlockInfo,
    test_utils::*,
};

#[test]
fn test_snark_account_deposit_and_withdrawal() {
    // Start with empty genesis state
    let mut state = create_test_genesis_state();

    // Create a snark account in the state
    let snark_account_id = get_test_snark_account_id();
    let initial_state_root = Hash::from([1u8; 32]);

    // Create a OLSnarkAccountState with always-accept predicate key for testing
    let vk = PredicateKey::always_accept();
    let snark_account_state = OLSnarkAccountState::new_fresh(vk, initial_state_root);

    let new_acct_data = NewAccountData::new_empty(AccountTypeState::Snark(snark_account_state));
    let snark_serial = state
        .create_new_account(snark_account_id, new_acct_data)
        .expect("Should create snark account");

    // Create a genesis block with a manifest containing a deposit to the snark account
    let deposit_amount = 150_000_000u64; // 1.5 BTC in satoshis (must be enough to cover withdrawal)
    let dest_subject = SubjectId::from([42u8; 32]);

    // Encode the destination as a DepositDescriptor (the wire format used on L1).
    let dest_subject_bytes =
        SubjectIdBytes::try_new(dest_subject.inner().to_vec()).expect("valid subject bytes");
    let descriptor =
        DepositDescriptor::new(snark_serial, dest_subject_bytes).expect("valid deposit descriptor");
    let destination = descriptor.encode_to_varvec();

    // Create a DepositLog matching what the bridge-v1 subprotocol actually emits.
    let deposit_log_data = DepositLog::new(destination, deposit_amount);
    let deposit_log =
        AsmLogEntry::from_log(&deposit_log_data).expect("Should create deposit log entry");

    // Create manifest with the deposit log
    let genesis_manifest = AsmManifest::new(
        1, // Genesis manifest should be at height 1 when last_l1_height is 0
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![deposit_log],
    )
    .expect("test manifest should be valid");

    // Execute genesis block with the deposit manifest
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis_output =
        execute_block_with_outputs(&mut state, &genesis_info, None, genesis_components)
            .expect("Genesis block should execute");
    let genesis_block = genesis_output.completed_block();

    // Verify the deposit was processed
    let (account_after_deposit, snark_state_after_genesis) =
        lookup_snark_account_states(&state, snark_account_id);
    assert_eq!(
        account_after_deposit.balance(),
        BitcoinAmount::from_sat(deposit_amount),
        "Account balance should reflect the deposit"
    );

    // Check inbox state after genesis
    let nxt_inbox_idx_after_gen = snark_state_after_genesis.next_inbox_msg_idx();
    // The deposit should have added a message to the inbox, but it hasn't been processed yet
    assert_eq!(
        nxt_inbox_idx_after_gen, 0,
        "Next inbox idx should still be zero (no messages processed yet)"
    );
    // Check how many messages are in the inbox
    let num_inbox_messages = snark_state_after_genesis.inbox_mmr().num_entries();
    assert_eq!(
        num_inbox_messages, 1,
        "Should have 1 deposit message in inbox after genesis"
    );
    debug!(
        "Inbox MMR has {num_inbox_messages} messages, next to process: {nxt_inbox_idx_after_gen}"
    );

    // Check the proof state (next message to PROCESS)
    let new_inner_st_root = snark_state_after_genesis.inner_state_root();
    debug!("New inner_state_root: {new_inner_st_root:?}");

    // Create parallel MMR tracker to generate proofs for the deposit message
    let mut inbox_tracker = InboxMmrTracker::new();

    // Track the deposit message that was added to the inbox during genesis processing
    // This message was added when the deposit intent log was processed
    let mut deposit_msg_data = Vec::new();
    let subject_bytes: [u8; 32] = dest_subject.into();
    deposit_msg_data.extend_from_slice(&subject_bytes);
    let deposit_msg_in_inbox = MessageEntry::new(
        BRIDGE_GATEWAY_ACCT_ID,
        0, // genesis epoch
        MsgPayload::new(BitcoinAmount::from_sat(deposit_amount), deposit_msg_data),
    );

    // Add the message to the tracker to get a proof
    let deposit_msg_proof = inbox_tracker.add_message(&deposit_msg_in_inbox);

    // Now create a snark account update transaction that produces a withdrawal
    let withdrawal_amount = 100_000_000u64; // Withdraw exactly 1 BTC (required denomination)
    let withdrawal_dest_desc = b"bc1qexample".to_vec(); // Example Bitcoin address descriptor
    let withdrawal_payload_data = encode_withdrawal_payload(&withdrawal_dest_desc, u32::MAX);

    // Build the snark update using SnarkUpdateBuilder
    let snark_account_state = lookup_snark_state(&state, snark_account_id);

    let sau_tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_processed_msgs(vec![deposit_msg_in_inbox])
        .with_inbox_proofs(vec![deposit_msg_proof])
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            withdrawal_amount,
            withdrawal_payload_data,
        )
        .build(
            snark_account_id,
            get_test_state_root(2),
            vec![0u8; 32], // Dummy proof for testing
        );

    // Create block 1 with the withdrawal transaction
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1_components = BlockComponents::new_txs_from_ol_transactions(vec![sau_tx]);
    let block1_output = execute_block_with_outputs(
        &mut state,
        &block1_info,
        Some(genesis_block.header()),
        block1_components,
    )
    .expect("Block 1 should execute");

    let _block1 = block1_output.completed_block();

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
    let mut withdrawal_found = false;

    for log in logs {
        // Check if it's a withdrawal intent log (from the bridge gateway)
        if log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL
            && let Ok(withdrawal_log) = strata_codec::decode_buf_exact::<
                strata_ol_chain_types_new::SimpleWithdrawalIntentLogData,
            >(log.payload())
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

            assert_eq!(
                withdrawal_log.selected_operator,
                u32::MAX,
                "Withdrawal operator should preserve the any-operator sentinel"
            );
        }
    }

    assert!(withdrawal_found, "test: missing withdrawal intent log");
}

#[test]
fn test_bridge_gateway_direct_transfer_is_silently_dropped() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        // Pins the current footgun: funds are deducted from sender but never delivered.
        .with_transfer(BRIDGE_GATEWAY_ACCT_ID, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = execute_single_tx_with_outputs(&mut state, genesis_block.header(), tx, 1, 1);

    assert_no_bridge_gateway_logs(&output);
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(90_000_000)
    );
    assert_eq!(*snark_account_state.seqno().inner(), 1);
}

#[test]
fn test_bridge_gateway_non_denomination_withdrawal_is_silently_dropped() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        // Pins the current footgun: funds are deducted from sender but never delivered.
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            50_000_000,
            encode_withdrawal_payload(b"bc1qnondenomination", u32::MAX),
        )
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = execute_single_tx_with_outputs(&mut state, genesis_block.header(), tx, 1, 1);

    assert_no_bridge_gateway_logs(&output);
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(50_000_000)
    );
    assert_eq!(*snark_account_state.seqno().inner(), 1);
}

#[test]
fn test_bridge_gateway_zero_amount_withdrawal_is_silently_dropped() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account_state = lookup_snark_state(&state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            0,
            encode_withdrawal_payload(b"bc1qzeroamount", u32::MAX),
        )
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = execute_single_tx_with_outputs(&mut state, genesis_block.header(), tx, 1, 1);

    assert_no_bridge_gateway_logs(&output);
    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(*snark_account_state.seqno().inner(), 1);
}

#[test]
fn test_bridge_gateway_non_withdrawal_gam_is_silently_dropped() {
    let mut state = create_test_genesis_state();
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_block = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("terminal genesis should execute");

    let tx = make_gam_tx(BRIDGE_GATEWAY_ACCT_ID);
    let output = execute_single_tx_with_outputs(&mut state, genesis_block.header(), tx, 1, 1);

    assert_no_bridge_gateway_logs(&output);
    assert!(
        !state
            .check_account_exists(BRIDGE_GATEWAY_ACCT_ID)
            .expect("account existence check should succeed"),
        "bridge gateway is a special account, not a ledger account"
    );
}

#[test]
fn test_bridge_gateway_wrong_msg_type_dropped() {
    // Properly framed message with a non-withdrawal type byte. The bridge
    // gateway parses the envelope successfully and then drops it because
    // `try_as_withdrawal` returns None — distinct from the empty-payload case
    // which bails at envelope parsing.
    let mut state = create_test_genesis_state();
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_block = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("terminal genesis should execute");

    let framed_payload = OwnedMsg::new(DEPOSIT_MSG_TYPE_ID, vec![])
        .expect("valid message framing should construct")
        .to_vec();
    let tx = make_gam_tx_with_payload(BRIDGE_GATEWAY_ACCT_ID, framed_payload);
    let output = execute_single_tx_with_outputs(&mut state, genesis_block.header(), tx, 1, 1);

    assert_no_bridge_gateway_logs(&output);
    assert!(
        !state
            .check_account_exists(BRIDGE_GATEWAY_ACCT_ID)
            .expect("account existence check should succeed"),
        "bridge gateway is a special account, not a ledger account"
    );
}

fn encode_withdrawal_payload(dest_desc: &[u8], selected_operator: u32) -> Vec<u8> {
    let withdrawal_msg_data =
        WithdrawalMsgData::new(DEFAULT_OPERATOR_FEE, dest_desc.to_vec(), selected_operator)
            .expect("valid withdrawal data");
    let encoded_withdrawal_body =
        strata_codec::encode_to_vec(&withdrawal_msg_data).expect("withdrawal data should encode");
    OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body)
        .expect("withdrawal message should be valid")
        .to_vec()
}

fn execute_single_tx_with_outputs(
    state: &mut OLState,
    parent_header: &OLBlockHeader,
    tx: OLTransaction,
    slot: u64,
    epoch: u32,
) -> ConstructBlockOutput {
    let block_info = BlockInfo::new(1_001_000, slot, epoch);
    let components = BlockComponents::new_txs_from_ol_transactions(vec![tx]);
    execute_block_with_outputs(state, &block_info, Some(parent_header), components)
        .expect("single-tx block should execute")
}

fn assert_no_bridge_gateway_logs(output: &ConstructBlockOutput) {
    assert!(
        output
            .outputs()
            .logs()
            .iter()
            .all(|log| log.account_serial() != BRIDGE_GATEWAY_ACCT_SERIAL),
        "bridge gateway should not emit withdrawal logs"
    );
}
