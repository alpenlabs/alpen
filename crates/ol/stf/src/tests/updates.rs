//! Tests for successful update operations

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::SnarkAccountUpdateLogData;

use crate::{SEQUENCER_ACCT_ID, assembly::BlockComponents, context::BlockInfo, test_utils::*};

#[test]
fn test_snark_update_success_with_transfer() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Setup: genesis with snark account
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Create recipient account
    create_empty_account(&mut state, recipient_id);

    // Create valid update with transfer
    let transfer_amount = 30_000_000u64;
    let tx = SnarkUpdateBuilder::from_snark_state(
        state
            .get_account_state(snark_id)
            .unwrap()
            .unwrap()
            .as_snark_account()
            .unwrap()
            .clone(),
    )
    .with_transfer(recipient_id, transfer_amount)
    .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let (slot, epoch) = (1, 0);
    let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
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
        "Sender account balance should be 100M - 30M"
    );
    // Check the seq no of the sender
    assert_eq!(
        *snark_account.as_snark_account().unwrap().seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
    assert_eq!(
        recipient_account.balance(),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient should receive 30M"
    );
}

#[test]
fn test_snark_update_emits_log() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_serial = snark_account.serial();
    let snark_state = snark_account.as_snark_account().unwrap().clone();
    let pre_msg_idx = snark_state.next_inbox_msg_idx();

    let extra_data = b"snark-update-extra".to_vec();
    let tx = SnarkUpdateBuilder::from_snark_state(snark_state)
        .try_with_extra_data(extra_data.clone())
        .expect("extra_data fits within SSZ bound")
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let block_info = BlockInfo::new(1_001_000, 1, 0);
    let components = BlockComponents::new_txs_from_ol_transactions(vec![tx]);
    let output = execute_block_with_outputs(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        components,
    )
    .expect("block should execute");

    let log = find_typed_log::<SnarkAccountUpdateLogData>(&output, snark_serial)
        .expect("snark update log not found");
    assert_eq!(
        log.new_msg_idx, pre_msg_idx,
        "no messages processed; new_msg_idx should equal pre-update next_inbox_msg_idx"
    );
    assert_eq!(log.extra_data.as_ref(), extra_data.as_slice());
}

#[test]
fn test_snark_update_emits_log_with_processed_message() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    // Block 1: deliver a GAM message into the snark account's inbox.
    let mut inbox_tracker = InboxMmrTracker::new();
    let gam_tx = make_gam_tx(snark_id);
    let (slot, epoch) = (1, 0);
    let blk1 = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch)
        .expect("GAM should succeed");

    // Mirror the message into the parallel MMR so we can produce a valid proof.
    let gam_msg = MessageEntry::new(SEQUENCER_ACCT_ID, epoch, MsgPayload::new_empty());
    let gam_proof = inbox_tracker.add_message(&gam_msg);

    // Capture pre-update state.
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    let snark_serial = snark_account.serial();
    let snark_state = snark_account.as_snark_account().unwrap().clone();
    let pre_msg_idx = snark_state.next_inbox_msg_idx();

    // Block 2: snark update that processes the inbox message.
    let update_tx = SnarkUpdateBuilder::from_snark_state(snark_state)
        .with_processed_msgs(vec![gam_msg])
        .with_inbox_proofs(vec![gam_proof])
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let block_info = BlockInfo::new(1_002_000, 2, 0);
    let components = BlockComponents::new_txs_from_ol_transactions(vec![update_tx]);
    let output =
        execute_block_with_outputs(&mut state, &block_info, Some(blk1.header()), components)
            .expect("update block should execute");

    let log = find_typed_log::<SnarkAccountUpdateLogData>(&output, snark_serial)
        .expect("snark update log not found");
    assert_eq!(
        log.new_msg_idx,
        pre_msg_idx + 1,
        "new_msg_idx should advance by the number of processed messages"
    );
    assert!(
        log.extra_data.is_empty(),
        "extra_data was not set on this update"
    );
}
