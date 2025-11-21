//! Unit tests for the OL STF implementation.

use strata_acct_types::{AccountId, VarVec};
use strata_ledger_types::{IGlobalState, IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::{GamTxPayload, TransactionPayload};
use strata_ol_state_types::OLState;

use crate::{
    assembly::BlockComponents,
    context::BlockInfo,
    test_utils::{
        assert_block_position, assert_state_updated, build_empty_chain, execute_block,
        test_account_id,
    },
};

#[test]
fn test_genesis_block_processing() {
    // Start from empty genesis state
    let mut state = OLState::new_genesis();

    // Verify initial state
    assert_eq!(state.l1_view().cur_epoch(), 0);
    assert_eq!(state.global_mut().cur_slot(), 0);

    // Process empty genesis block
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_block = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )
    .expect("Genesis block execution should succeed");

    // Verify genesis block header
    assert_block_position(genesis_block.header(), 0, 0);
    assert_eq!(genesis_block.header().timestamp(), 1000000);

    // State should still be at epoch 0, slot 0 after processing genesis
    assert_state_updated(&mut state, 0, 0);

    // Verify state root was computed
    let state_root = state
        .compute_state_root()
        .expect("State root computation should succeed");
    assert_eq!(genesis_block.header().state_root(), &state_root);
}

#[test]
fn test_post_genesis_blocks() {
    let mut state = OLState::new_genesis();

    // Process genesis block
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )
    .expect("Genesis block should execute");

    // Process block at slot 1
    let block1_info = BlockInfo::new(1001000, 1, 0);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 should execute");

    assert_block_position(block1.header(), 0, 1);
    assert_state_updated(&mut state, 0, 1);

    // Process block at slot 2
    let block2_info = BlockInfo::new(1002000, 2, 0);
    let block2 = execute_block(
        &mut state,
        &block2_info,
        Some(block1.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 2 should execute");

    assert_block_position(block2.header(), 0, 2);
    assert_state_updated(&mut state, 0, 2);

    // Verify parent hash chaining - blocks should reference their parent
    // Note: We can't easily verify the parent chain without computing block IDs
}

#[test]
fn test_genesis_with_initial_transactions() {
    let mut state = OLState::new_genesis();

    // Create some test transactions for genesis
    let target = test_account_id(1);
    let msg = b"Hello from genesis".to_vec();
    let msg_varvec = VarVec::from_vec(msg).expect("VarVec creation should succeed");

    let tx = TransactionPayload::GenericAccountMessage(GamTxPayload::new(target, msg_varvec));

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_txs(vec![tx.clone()]),
    )
    .expect("Genesis with transactions should execute");

    // Verify block was created at correct position
    assert_block_position(genesis.header(), 0, 0);

    // Verify body contains the transaction
    assert_eq!(genesis.body().tx_segment().txs().len(), 1);
    // We'd need to access the transaction payload through OLTransaction to verify
}

#[test]
fn test_epoch_transition_from_genesis() {
    let mut state = OLState::new_genesis();
    const SLOTS_PER_EPOCH: u64 = 10;

    // Build a chain through the first epoch transition
    let headers =
        build_empty_chain(&mut state, 12, SLOTS_PER_EPOCH).expect("Chain building should succeed");

    // Verify we have the expected number of blocks
    assert_eq!(headers.len(), 12);

    // Check genesis block
    assert_block_position(&headers[0], 0, 0);

    // Check last block of epoch 0
    assert_block_position(&headers[9], 0, 9);

    // Check first block of epoch 1
    assert_block_position(&headers[10], 1, 10);

    // Check another block in epoch 1
    assert_block_position(&headers[11], 1, 11);

    // Verify final state
    assert_state_updated(&mut state, 1, 11);
}

#[test]
fn test_empty_chain_building() {
    let mut state = OLState::new_genesis();

    // Build a chain of 5 empty blocks
    let headers =
        build_empty_chain(&mut state, 5, 100).expect("Building empty chain should succeed");

    assert_eq!(headers.len(), 5);

    // Verify slots increment properly
    for (i, header) in headers.iter().enumerate() {
        assert_eq!(header.slot(), i as u64);
        assert_eq!(header.epoch(), 0); // All in epoch 0 since slots_per_epoch=100
    }

    // Verify final state
    assert_state_updated(&mut state, 0, 4);
}

#[test]
fn test_state_persistence_across_blocks() {
    let mut state = OLState::new_genesis();

    // Process genesis
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )
    .expect("Genesis should execute");

    // Get state root after genesis
    let genesis_state_root = state
        .compute_state_root()
        .expect("State root should compute");

    // Process another empty block
    let block1_info = BlockInfo::new(1001000, 1, 0);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 should execute");

    // State root should change due to slot update
    let block1_state_root = state
        .compute_state_root()
        .expect("State root should compute");

    assert_ne!(
        genesis_state_root, block1_state_root,
        "State root should change between blocks"
    );

    // Verify the roots match what was recorded in blocks
    assert_eq!(*genesis.header().state_root(), genesis_state_root);
    assert_eq!(*block1.header().state_root(), block1_state_root);
}
