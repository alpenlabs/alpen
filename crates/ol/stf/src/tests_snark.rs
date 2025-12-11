//! Test for snark account deposit and withdrawal

use strata_acct_types::{BitcoinAmount, Hash, MsgPayload};
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
use strata_ol_state_types::{NativeSnarkAccountState, OLState};
use strata_snark_acct_types::{
    LedgerRefProofs, LedgerRefs, OutputMessage, ProofState, SnarkAccountUpdate,
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

#[test]
fn test_snark_account_deposit_and_withdrawal() {
    // Start with empty genesis state
    let mut state = OLState::new_genesis();

    // Create a snark account in the state
    let snark_account_id = test_account_id(100);
    let initial_state_root = Hash::from([1u8; 32]);

    // Create a NativeSnarkAccountState using the public new_fresh method
    let snark_state = NativeSnarkAccountState::new_fresh(initial_state_root);

    let new_acct_data = NewAccountData::new_empty(AccountTypeState::Snark(snark_state));
    let snark_serial = state
        .create_new_account(snark_account_id, new_acct_data)
        .expect("Should create snark account");

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
    let new_seqno = 1u64; // Next sequence number
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
