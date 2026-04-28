//! Staging-layer tests for OL STF transaction and verification entry points.
//!
//! These tests cover `WriteTrackingState` directly for tx execution and
//! `IndexerState<WriteTrackingState<_>>` for verification-side accumulator writes.

use std::collections::BTreeSet;

use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload};
use strata_ledger_types::{
    AccountTypeState, IAccountState, ISnarkAccountState, IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::OLTxSegment;
use strata_ol_state_support_types::{IndexerState, WriteTrackingState};
use strata_ol_state_types::OLState;

use crate::{
    assembly::BlockComponents,
    context::{BasicExecContext, BlockInfo, TxExecContext},
    errors::ExecError,
    output::ExecOutputBuffer,
    test_utils::*,
    transaction_processing::{process_block_tx_segment, process_single_tx},
    verification::verify_block,
};

#[test]
fn test_process_single_tx_stages_sau_writes() {
    let mut base_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut base_state, recipient_id);
    let genesis = setup_genesis_with_snark_account(&mut base_state, snark_id, 100_000_000);
    let base_root = base_state
        .compute_state_root()
        .expect("base root should compute");

    let snark_state = lookup_snark_state(&base_state, snark_id);
    let tx = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = ExecOutputBuffer::new_empty();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let basic_context = BasicExecContext::new(block_info, &output);
    let tx_context = TxExecContext::new(&basic_context, Some(genesis.header()));
    let mut tracking = WriteTrackingState::new_from_state(&base_state);

    process_single_tx(&mut tracking, &tx, &tx_context).expect("SAU transfer should stage");

    let staged_sender = tracking
        .get_account_state(snark_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist");
    let staged_sender_snark = staged_sender
        .as_snark_account()
        .expect("sender should be snark");
    assert_eq!(staged_sender.balance(), BitcoinAmount::from_sat(90_000_000));
    assert_eq!(*staged_sender_snark.seqno().inner(), 1);
    assert_eq!(
        staged_sender_snark.inner_state_root(),
        get_test_state_root(2)
    );

    let staged_recipient = tracking
        .get_account_state(recipient_id)
        .expect("recipient lookup should succeed")
        .expect("recipient should exist");
    assert_eq!(
        staged_recipient.balance(),
        BitcoinAmount::from_sat(10_000_000)
    );

    let base_sender = base_state
        .get_account_state(snark_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist");
    assert_eq!(base_sender.balance(), BitcoinAmount::from_sat(100_000_000));
    assert_eq!(
        base_state
            .compute_state_root()
            .expect("base root should compute"),
        base_root
    );

    let mut applied_state = base_state.clone();
    applied_state
        .apply_write_batch(tracking.into_batch())
        .expect("staged batch should apply");
    assert_eq!(
        applied_state
            .get_account_state(snark_id)
            .expect("sender lookup should succeed")
            .expect("sender should exist")
            .balance(),
        BitcoinAmount::from_sat(90_000_000)
    );
}

fn assert_transfer_staged(
    tracking: &WriteTrackingState<'_, OLState>,
    snark_id: AccountId,
    recipient_id: AccountId,
    expected_sender_balance: u64,
    expected_recipient_balance: u64,
    expected_seqno: u64,
) {
    let staged_sender = tracking
        .get_account_state(snark_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist");
    assert_eq!(
        staged_sender.balance(),
        BitcoinAmount::from_sat(expected_sender_balance)
    );
    assert_eq!(
        *staged_sender
            .as_snark_account()
            .expect("sender should be snark")
            .seqno()
            .inner(),
        expected_seqno
    );
    assert_eq!(
        tracking
            .get_account_state(recipient_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance(),
        BitcoinAmount::from_sat(expected_recipient_balance)
    );
}

#[test]
fn test_process_single_tx_reads_prior_staged_writes() {
    let mut base_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut base_state, recipient_id);
    let genesis = setup_genesis_with_snark_account(&mut base_state, snark_id, 100_000_000);

    let snark_state = lookup_snark_state(&base_state, snark_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = ExecOutputBuffer::new_empty();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let basic_context = BasicExecContext::new(block_info, &output);
    let tx_context = TxExecContext::new(&basic_context, Some(genesis.header()));
    let mut tracking = WriteTrackingState::new_from_state(&base_state);

    process_single_tx(&mut tracking, &tx1, &tx_context).expect("first SAU should stage");
    assert_transfer_staged(&tracking, snark_id, recipient_id, 90_000_000, 10_000_000, 1);

    let err = process_single_tx(&mut tracking, &tx1, &tx_context)
        .expect_err("replayed SAU should fail against the staged seqno");
    match err.base() {
        ExecError::Acct(AcctError::InvalidUpdateSequence {
            account_id,
            expected,
            got,
        }) => {
            assert_eq!(*account_id, snark_id);
            assert_eq!(*expected, 1);
            assert_eq!(*got, 0);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }
    assert_transfer_staged(&tracking, snark_id, recipient_id, 90_000_000, 10_000_000, 1);

    let staged_snark = tracking
        .get_account_state(snark_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist")
        .as_snark_account()
        .expect("sender should be snark")
        .clone();
    let tx2 = SnarkUpdateBuilder::from_snark_state(staged_snark)
        .with_transfer(recipient_id, 5_000_000)
        .build(snark_id, get_test_state_root(3), get_test_proof(2));

    process_single_tx(&mut tracking, &tx2, &tx_context)
        .expect("second SAU should read staged state");
    assert_transfer_staged(&tracking, snark_id, recipient_id, 85_000_000, 15_000_000, 2);

    assert_eq!(
        base_state
            .get_account_state(snark_id)
            .expect("sender lookup should succeed")
            .expect("sender should exist")
            .balance(),
        BitcoinAmount::from_sat(100_000_000)
    );
}

#[test]
fn test_process_single_tx_loop_can_restore_failed_tx_batch() {
    let mut base_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut base_state, recipient_id);
    let genesis = setup_genesis_with_snark_account(&mut base_state, snark_id, 100_000_000);

    let snark_state = lookup_snark_state(&base_state, snark_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = ExecOutputBuffer::new_empty();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let basic_context = BasicExecContext::new(block_info, &output);
    let tx_context = TxExecContext::new(&basic_context, Some(genesis.header()));
    let mut tracking = WriteTrackingState::new_from_state(&base_state);

    let pre_tx1_root = tracking
        .compute_state_root()
        .expect("pre-tx root should compute");
    process_single_tx(&mut tracking, &tx1, &tx_context).expect("first SAU should stage");
    assert_ne!(
        tracking
            .compute_state_root()
            .expect("post-tx root should compute"),
        pre_tx1_root
    );
    assert_transfer_staged(&tracking, snark_id, recipient_id, 90_000_000, 10_000_000, 1);

    let before_failed_tx = tracking.batch().clone();
    let post_tx1_root = tracking
        .compute_state_root()
        .expect("pre-failed-tx root should compute");
    let err = process_single_tx(&mut tracking, &tx1, &tx_context)
        .expect_err("replayed SAU should fail against the staged seqno");
    assert!(matches!(
        err.base(),
        ExecError::Acct(AcctError::InvalidUpdateSequence { .. })
    ));
    // Mirrors the per-tx rollback dance in block_assembly.rs after a failed tx.
    tracking = WriteTrackingState::new(&base_state, before_failed_tx.clone());
    assert_eq!(
        tracking
            .compute_state_root()
            .expect("restored root should compute"),
        post_tx1_root
    );
    assert_transfer_staged(&tracking, snark_id, recipient_id, 90_000_000, 10_000_000, 1);

    let staged_snark = tracking
        .get_account_state(snark_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist")
        .as_snark_account()
        .expect("sender should be snark")
        .clone();
    let tx2 = SnarkUpdateBuilder::from_snark_state(staged_snark)
        .with_transfer(recipient_id, 5_000_000)
        .build(snark_id, get_test_state_root(3), get_test_proof(2));

    process_single_tx(&mut tracking, &tx2, &tx_context)
        .expect("second SAU should continue after restoring failed tx");
    assert_transfer_staged(&tracking, snark_id, recipient_id, 85_000_000, 15_000_000, 2);
}

#[test]
fn test_process_tx_segment_reads_staged_writes_between_txs() {
    let mut base_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut base_state, recipient_id);
    let genesis = setup_genesis_with_snark_account(&mut base_state, snark_id, 100_000_000);

    let snark_state = lookup_snark_state(&base_state, snark_id);
    let tx1 = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let mut expected_after_tx1 = base_state.clone();
    execute_tx_in_block(&mut expected_after_tx1, genesis.header(), tx1.clone(), 1, 1)
        .expect("first SAU should execute on expected state");
    let staged_snark = lookup_snark_state(&expected_after_tx1, snark_id);
    let tx2 = SnarkUpdateBuilder::from_snark_state(staged_snark.clone())
        .with_transfer(recipient_id, 5_000_000)
        .build(snark_id, get_test_state_root(3), get_test_proof(2));

    let output = ExecOutputBuffer::new_empty();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let basic_context = BasicExecContext::new(block_info, &output);
    let tx_context = TxExecContext::new(&basic_context, Some(genesis.header()));
    let mut tracking = WriteTrackingState::new_from_state(&base_state);
    let tx_segment = OLTxSegment::new(vec![tx1, tx2]).expect("tx segment should fit");

    process_block_tx_segment(&mut tracking, &tx_segment, &tx_context)
        .expect("tx segment should read staged writes between txs");

    assert_transfer_staged(&tracking, snark_id, recipient_id, 85_000_000, 15_000_000, 2);
    assert_eq!(
        tracking.cur_slot(),
        base_state.cur_slot(),
        "process_block_tx_segment does not run block-start processing"
    );

    let mut applied_state = base_state.clone();
    applied_state
        .apply_write_batch(tracking.into_batch())
        .expect("staged segment batch should apply");
    assert_eq!(
        lookup_snark_state(&applied_state, snark_id).inner_state_root(),
        get_test_state_root(3)
    );
    assert_eq!(
        applied_state
            .get_account_state(recipient_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance(),
        BitcoinAmount::from_sat(15_000_000)
    );
}

#[test]
fn test_write_tracking_stages_account_creation_before_apply() {
    let base_state = create_test_genesis_state();
    let new_account_id = test_account_id(77);
    let mut tracking = WriteTrackingState::new_from_state(&base_state);

    let expected_serial = base_state.next_account_serial();
    let serial = tracking
        .create_new_account(
            new_account_id,
            NewAccountData::new_empty(AccountTypeState::Empty),
        )
        .expect("account creation should stage");
    assert_eq!(serial, expected_serial);
    assert!(!base_state.check_account_exists(new_account_id).unwrap());

    assert_eq!(tracking.batch().ledger().new_accounts(), &[new_account_id]);
    assert_eq!(
        tracking
            .batch()
            .ledger()
            .iter_new_accounts()
            .collect::<Vec<_>>(),
        vec![(expected_serial, &new_account_id)]
    );
    let staged_account = tracking
        .get_account_state(new_account_id)
        .expect("staged lookup should succeed")
        .expect("staged account should be readable");
    assert_eq!(staged_account.serial(), expected_serial);
    assert_eq!(staged_account.balance(), BitcoinAmount::zero());

    let mut applied_state = base_state.clone();
    applied_state
        .apply_write_batch(tracking.into_batch())
        .expect("account-creation batch should apply");
    assert!(applied_state.check_account_exists(new_account_id).unwrap());
}

#[test]
fn test_multi_effect_sau_coalesces_staged_account_writes() {
    let mut base_state = create_test_genesis_state();
    let sender_id = get_test_snark_account_id();
    let recipient1_id = get_test_recipient_account_id();
    let recipient2_id = test_account_id(88);
    let message_recipient_id = test_account_id(89);
    create_empty_account(&mut base_state, recipient1_id);
    create_empty_account(&mut base_state, recipient2_id);
    let genesis = setup_genesis_with_snark_accounts(
        &mut base_state,
        &[(sender_id, 100_000_000), (message_recipient_id, 0)],
    );

    let sender_state = lookup_snark_state(&base_state, sender_id);
    let tx = SnarkUpdateBuilder::from_snark_state(sender_state.clone())
        .with_transfer(recipient1_id, 10_000_000)
        .with_transfer(recipient2_id, 20_000_000)
        .with_output_message(message_recipient_id, 1, b"first".to_vec())
        .with_output_message(message_recipient_id, 2, b"second".to_vec())
        .build(sender_id, get_test_state_root(2), get_test_proof(1));

    let output = ExecOutputBuffer::new_empty();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let basic_context = BasicExecContext::new(block_info, &output);
    let tx_context = TxExecContext::new(&basic_context, Some(genesis.header()));
    let mut tracking = WriteTrackingState::new_from_state(&base_state);

    process_single_tx(&mut tracking, &tx, &tx_context).expect("multi-effect SAU should stage");

    let written_accounts = tracking
        .batch()
        .ledger()
        .iter_accounts()
        .map(|(account_id, _)| *account_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        written_accounts,
        BTreeSet::from([
            sender_id,
            recipient1_id,
            recipient2_id,
            message_recipient_id,
        ])
    );

    let staged_sender = tracking
        .get_account_state(sender_id)
        .expect("sender lookup should succeed")
        .expect("sender should exist");
    assert_eq!(staged_sender.balance(), BitcoinAmount::from_sat(69_999_997));
    let staged_sender_snark = staged_sender
        .as_snark_account()
        .expect("sender should be snark");
    assert_eq!(*staged_sender_snark.seqno().inner(), 1);
    assert_eq!(
        staged_sender_snark.inner_state_root(),
        get_test_state_root(2)
    );

    assert_eq!(
        tracking
            .get_account_state(recipient1_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance(),
        BitcoinAmount::from_sat(10_000_000)
    );
    assert_eq!(
        tracking
            .get_account_state(recipient2_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance(),
        BitcoinAmount::from_sat(20_000_000)
    );
    let message_recipient = tracking
        .get_account_state(message_recipient_id)
        .expect("message recipient lookup should succeed")
        .expect("message recipient should exist");
    assert_eq!(message_recipient.balance(), BitcoinAmount::from_sat(3));
    assert_eq!(
        message_recipient
            .as_snark_account()
            .expect("message recipient should be snark")
            .inbox_mmr()
            .num_entries(),
        2
    );
}

#[test]
fn test_assembly_and_verify_write_tracking_reach_same_state() {
    let mut assembly_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();
    create_empty_account(&mut assembly_state, recipient_id);
    let genesis = setup_genesis_with_snark_account(&mut assembly_state, snark_id, 100_000_000);
    let pre_block_state = assembly_state.clone();

    let snark_state = lookup_snark_state(&assembly_state, snark_id);
    let gam_tx = make_gam_tx_with_payload(snark_id, b"assembly verify equivalence".to_vec());
    let sau_tx = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_transfer(recipient_id, 10_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let block = execute_block(
        &mut assembly_state,
        &block_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![gam_tx, sau_tx]),
    )
    .expect("tx-bearing block should assemble");

    let tracking = WriteTrackingState::new_from_state(&pre_block_state);
    let mut indexer = IndexerState::new(tracking);
    verify_block(
        &mut indexer,
        block.header(),
        Some(genesis.header()),
        block.body(),
    )
    .expect("assembled block should verify through staging layers");

    let (tracking, writes) = indexer.into_parts();
    assert_eq!(writes.inbox_messages().len(), 1);

    let mut verified_state = pre_block_state.clone();
    verified_state
        .apply_write_batch(tracking.into_batch())
        .expect("verification batch should apply");

    assert_eq!(
        verified_state
            .compute_state_root()
            .expect("verified state root should compute"),
        assembly_state
            .compute_state_root()
            .expect("assembly state root should compute")
    );
    assert_eq!(
        verified_state
            .get_account_state(snark_id)
            .expect("snark lookup should succeed")
            .expect("snark should exist")
            .balance(),
        assembly_state
            .get_account_state(snark_id)
            .expect("snark lookup should succeed")
            .expect("snark should exist")
            .balance()
    );
    assert_eq!(
        verified_state
            .get_account_state(recipient_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance(),
        assembly_state
            .get_account_state(recipient_id)
            .expect("recipient lookup should succeed")
            .expect("recipient should exist")
            .balance()
    );
}

#[test]
fn test_verify_block_tracks_snark_inbox_writes() {
    let mut assembly_state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis = setup_genesis_with_snark_account(&mut assembly_state, snark_id, 100_000_000);
    let verify_base = assembly_state.clone();

    let payload = b"tracked inbox message".to_vec();
    let gam_tx = make_gam_tx_with_payload(snark_id, payload.clone());
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let block = execute_block(
        &mut assembly_state,
        &block_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![gam_tx]),
    )
    .expect("GAM block should execute");

    let tracking = WriteTrackingState::new_from_state(&verify_base);
    let mut indexer = IndexerState::new(tracking);
    verify_block(
        &mut indexer,
        block.header(),
        Some(genesis.header()),
        block.body(),
    )
    .expect("GAM block should verify");

    let (tracking, writes) = indexer.into_parts();
    assert_eq!(writes.inbox_messages().len(), 1);
    let inbox_write = &writes.inbox_messages()[0];
    assert_eq!(inbox_write.account_id(), snark_id);
    assert_eq!(inbox_write.index(), 0);
    assert_eq!(inbox_write.entry().incl_epoch(), 1);
    assert_eq!(
        inbox_write.entry().payload(),
        &MsgPayload::new_valueless(payload)
    );

    let staged_snark = tracking
        .get_account_state(snark_id)
        .expect("snark lookup should succeed")
        .expect("snark should exist")
        .as_snark_account()
        .expect("account should be snark");
    assert_eq!(staged_snark.inbox_mmr().num_entries(), 1);
}

#[test]
fn test_verify_block_through_write_tracking_stack() {
    // This test mimics chain-worker-new's verification path:
    // IndexerState<WriteTrackingState<&OLState>> with verify_block
    let mut state = create_test_genesis_state();

    // Assemble genesis block (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    // Assemble block 1 (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Now verify using the WriteTrackingState + IndexerState stack,
    // same composition as chain-worker-new.
    let verify_base = create_test_genesis_state();

    // Verify genesis through the stack
    {
        let tracking = WriteTrackingState::new_from_state(&verify_base);
        let mut indexer = IndexerState::new(tracking);

        verify_block(&mut indexer, genesis.header(), None, genesis.body())
            .expect("Genesis verification through write-tracking stack should succeed");
    }

    // Apply genesis writes to get post-genesis state for next block
    let mut post_genesis = verify_base.clone();
    {
        let tracking = WriteTrackingState::new_from_state(&post_genesis);
        let mut indexer = IndexerState::new(tracking);

        verify_block(&mut indexer, genesis.header(), None, genesis.body())
            .expect("Genesis verification should succeed");

        let (tracking, _writes) = indexer.into_parts();
        post_genesis
            .apply_write_batch(tracking.into_batch())
            .expect("Applying genesis batch should succeed");
    }

    // Verify block 1 through the stack using post-genesis state
    {
        let tracking = WriteTrackingState::new_from_state(&post_genesis);
        let mut indexer = IndexerState::new(tracking);

        verify_block(
            &mut indexer,
            block1.header(),
            Some(genesis.header()),
            block1.body(),
        )
        .expect("Block 1 verification through write-tracking stack should succeed");
    }
}

#[test]
fn test_verify_terminal_block_through_write_tracking_stack() {
    // Terminal blocks are important because verify_block calls compute_state_root twice
    // (pre-manifest and post-manifest), and the root changes between calls.
    let mut state = create_test_genesis_state();
    const SLOTS_PER_EPOCH: u64 = 3;

    // Build chain: genesis (terminal) + slots 1,2,3 where slot 3 is terminal
    let blocks =
        build_empty_chain(&mut state, 4, SLOTS_PER_EPOCH).expect("Chain building should succeed");

    assert!(
        blocks[0].header().is_terminal(),
        "Genesis should be terminal"
    );
    assert!(
        blocks[3].header().is_terminal(),
        "Block at slot 3 should be terminal"
    );

    // Verify the entire chain through WriteTrackingState stack
    let mut verify_base = create_test_genesis_state();

    for (i, block) in blocks.iter().enumerate() {
        let parent_header = if i == 0 {
            None
        } else {
            Some(blocks[i - 1].header().clone())
        };

        let tracking = WriteTrackingState::new_from_state(&verify_base);
        let mut indexer = IndexerState::new(tracking);

        verify_block(&mut indexer, block.header(), parent_header.as_ref(), block.body()).unwrap_or_else(
            |e| {
                panic!(
                    "Block {} (slot {}, terminal={}) verification through write-tracking stack failed: {:?}",
                    i,
                    block.header().slot(),
                    block.header().is_terminal(),
                    e
                )
            },
        );

        // Apply writes to advance state for next block
        let (tracking, _writes) = indexer.into_parts();
        verify_base
            .apply_write_batch(tracking.into_batch())
            .expect("Applying batch should succeed");
    }

    // Final state should match what assembly produced
    assert_eq!(state.cur_epoch(), verify_base.cur_epoch());
    assert_eq!(state.cur_slot(), verify_base.cur_slot());
    assert_eq!(
        state.compute_state_root().unwrap(),
        verify_base.compute_state_root().unwrap()
    );
}
