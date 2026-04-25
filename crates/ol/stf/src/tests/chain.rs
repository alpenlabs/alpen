//! Tests for chain lifecycle behavior (genesis, progression, and epoch transitions).

use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, L1BlockId, WtxidsRoot};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::*;

use crate::{assembly::BlockComponents, context::BlockInfo, test_utils::*};

#[test]
fn test_genesis_block_processing() {
    // Start from empty genesis state
    let mut state = create_test_genesis_state();

    // Verify initial state
    assert_eq!(state.cur_epoch(), 0);
    assert_eq!(state.cur_slot(), 0);

    // Process genesis block (with manifest to make it terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_block = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis block execution should succeed");

    // Verify genesis block header
    assert_block_position(genesis_block.header(), 0, 0);
    assert_eq!(genesis_block.header().timestamp(), 1000000);

    // State should be at epoch 1, slot 0 after processing terminal genesis
    assert_state_updated(&mut state, 1, 0);

    // Verify state root was computed
    let state_root = state
        .compute_state_root()
        .expect("State root computation should succeed");
    assert_eq!(genesis_block.header().state_root(), &state_root);

    // ADDITIONAL VERIFICATION: Verify the block passes verification
    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(
        &mut verify_state,
        genesis_block.header(),
        None,
        genesis_block.body(),
    );
}

#[test]
fn test_post_genesis_blocks() {
    let mut state = create_test_genesis_state();

    // Process genesis block (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis block should execute");

    // Process block at slot 1 (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 should execute");

    assert_block_position(block1.header(), 1, 1);
    assert_state_updated(&mut state, 1, 1);

    // Process block at slot 2 (still epoch 1)
    let block2_info = BlockInfo::new(1002000, 2, 1);
    let block2 = execute_block(
        &mut state,
        &block2_info,
        Some(block1.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 2 should execute");

    assert_block_position(block2.header(), 1, 2);
    assert_state_updated(&mut state, 1, 2);

    // ADDITIONAL VERIFICATION: Verify all blocks in the chain
    let mut verify_state = create_test_genesis_state();

    // Verify genesis
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Verify block 1
    assert_verification_succeeds(
        &mut verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );

    // Verify block 2
    assert_verification_succeeds(
        &mut verify_state,
        block2.header(),
        Some(block1.header().clone()),
        block2.body(),
    );
}

#[test]
fn test_genesis_with_initial_transactions() {
    let mut state = create_test_genesis_state();

    // Create some test transactions for genesis
    let target = test_account_id(1);
    let gam_tx = make_gam_tx(target);

    // Create genesis components with both transactions and manifest (to make it terminal)
    let dummy_manifest = AsmManifest::new(
        1, // Genesis manifest should be at height 1 when last_l1_height is 0
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    )
    .expect("test manifest should be valid");
    let genesis_components = BlockComponents::new(
        OLTxSegment::new(vec![gam_tx.clone()]).expect("tx segment should be within limits"),
        Some(
            OLL1ManifestContainer::new(vec![dummy_manifest])
                .expect("single manifest should succeed"),
        ),
    );

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_components)
        .expect("Genesis with transactions should execute");

    // Verify block was created at correct position
    assert_block_position(genesis.header(), 0, 0);

    // Verify body contains the transaction
    assert_eq!(
        genesis
            .body()
            .tx_segment()
            .expect("genesis should have tx_segment")
            .txs()
            .len(),
        1
    );

    // ADDITIONAL VERIFICATION: Verify the block with transactions passes verification
    let mut verify_state = create_test_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
}

#[test]
fn test_epoch_transition_from_genesis() {
    let mut state = create_test_genesis_state();
    const SLOTS_PER_EPOCH: u64 = 10;

    // Build a chain through the first epoch transition
    let headers = build_empty_chain_headers(&mut state, 12, SLOTS_PER_EPOCH)
        .expect("Chain building should succeed");

    // Verify we have the expected number of blocks
    assert_eq!(headers.len(), 12);

    // Check genesis block (should be terminal)
    assert_block_position(&headers[0], 0, 0);
    assert!(headers[0].is_terminal(), "Genesis should be terminal");

    // Check first block of epoch 1 (should not be terminal)
    assert_block_position(&headers[1], 1, 1);
    assert!(!headers[1].is_terminal(), "Block 1 should not be terminal");

    // Check last block of epoch 1 (should be terminal)
    assert_block_position(&headers[10], 1, 10);
    assert!(headers[10].is_terminal(), "Block 10 should be terminal");

    // Check first block of epoch 2
    assert_block_position(&headers[11], 2, 11);
    assert!(
        !headers[11].is_terminal(),
        "Block 11 should not be terminal"
    );

    // Verify final state (block 11 is in epoch 2)
    assert_state_updated(&mut state, 2, 11);
}

#[test]
fn test_empty_chain_building() {
    let mut state = create_test_genesis_state();

    // Build a chain of 5 empty blocks
    let headers =
        build_empty_chain_headers(&mut state, 5, 100).expect("Building empty chain should succeed");

    assert_eq!(headers.len(), 5);

    // Verify slots increment properly
    for (i, header) in headers.iter().enumerate() {
        assert_eq!(header.slot(), i as u64);
        // With genesis as terminal:
        // Slot 0 (genesis) is epoch 0
        // Slots 1-4 are epoch 1 (since slots_per_epoch=100)
        let expected_epoch = if i == 0 { 0 } else { 1 };
        assert_eq!(header.epoch(), expected_epoch);
    }

    // Verify final state
    // Genesis (terminal) increments epoch to 1, so state should be at epoch 1
    assert_state_updated(&mut state, 1, 4);
}

#[test]
fn test_state_persistence_across_blocks() {
    let mut state = create_test_genesis_state();

    // Process genesis (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis should execute");

    // Get state root after genesis
    let genesis_state_root = state
        .compute_state_root()
        .expect("State root should compute");

    // Process another empty block (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
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

    // ADDITIONAL VERIFICATION: Verify that the blocks can be verified
    let mut verify_state = create_test_genesis_state();

    // Verify genesis
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Verify block 1
    assert_verification_succeeds(
        &mut verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );
}

#[test]
fn test_process_chain_with_multiple_epochs() {
    // Test that we can process a chain from genesis through epoch 3
    // with 3 blocks per epoch (epochs 0, 1, 2, 3 = 12 blocks total)
    let mut state = create_test_genesis_state();
    const BLOCKS_PER_EPOCH: u64 = 3;
    const _TARGET_EPOCH: u32 = 3;
    const TOTAL_BLOCKS: usize = 12; // 4 epochs * 3 blocks each

    let mut blocks = Vec::new();
    let mut headers = Vec::new();

    // Build the entire chain
    for block_num in 0..TOTAL_BLOCKS {
        let slot = block_num as u64;
        // With genesis as terminal: epoch 0 is just slot 0, then epochs are 3 slots each
        let epoch = if block_num == 0 {
            0 // Genesis is epoch 0
        } else {
            ((slot - 1) / BLOCKS_PER_EPOCH + 1) as u32 // Slots 1-3 are epoch 1, 4-6 are epoch 2, etc.
        };
        let timestamp = 1000000 + (block_num as u64 * 1000);

        let block_info = if block_num == 0 {
            BlockInfo::new_genesis(timestamp)
        } else {
            BlockInfo::new(timestamp, slot, epoch)
        };

        let parent_header = if block_num == 0 {
            None
        } else {
            Some(&headers[block_num - 1])
        };

        // Determine if this is a terminal block
        // With genesis as terminal: slot 0 (genesis), 3, 6, 9 are terminal
        // This means epoch 0 has 1 block, then each epoch has 3 blocks
        let is_terminal = if block_num == 0 {
            true // Genesis is always terminal
        } else {
            slot.is_multiple_of(BLOCKS_PER_EPOCH) // Slots 3, 6, 9, etc. are terminal
        };

        let components = if is_terminal {
            // Create a terminal block with a dummy manifest
            let dummy_manifest = AsmManifest::new(
                state.last_l1_height() + 1, // Next L1 height after state's last seen
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            )
            .expect("test manifest should be valid");
            BlockComponents::new_manifests(vec![dummy_manifest])
        } else {
            BlockComponents::new_empty()
        };

        let Ok(block) = execute_block(&mut state, &block_info, parent_header, components) else {
            panic!("test: block {block_num} (epoch {epoch}, slot {slot}) should execute properly");
        };

        // Verify block position
        assert_block_position(block.header(), epoch as u64, slot);

        // Verify terminal flag is set correctly
        assert_eq!(
            block.header().is_terminal(),
            is_terminal,
            "Block {} terminal flag mismatch (expected: {}, actual: {})",
            block_num,
            is_terminal,
            block.header().is_terminal()
        );

        headers.push(block.header().clone());
        blocks.push(block);
    }

    // Verify we reached epoch 4 (slots 10-11 are in epoch 4)
    assert_eq!(headers.last().unwrap().epoch(), 4);
    // Block 11 is not terminal, so state epoch should be 4
    assert_state_updated(&mut state, 4, (TOTAL_BLOCKS - 1) as u64);

    // Verify epoch boundaries are correct
    // Epoch 0: slot 0 only (genesis)
    assert_eq!(headers[0].epoch(), 0);

    // Epoch 1: slots 1-3
    assert_eq!(headers[1].epoch(), 1);
    assert_eq!(headers[2].epoch(), 1);
    assert_eq!(headers[3].epoch(), 1);

    // Epoch 2: slots 4-6
    assert_eq!(headers[4].epoch(), 2);
    assert_eq!(headers[5].epoch(), 2);
    assert_eq!(headers[6].epoch(), 2);

    // Epoch 3: slots 7-9
    assert_eq!(headers[7].epoch(), 3);
    assert_eq!(headers[8].epoch(), 3);
    assert_eq!(headers[9].epoch(), 3);

    // Epoch 4: slots 10-11
    assert_eq!(headers[10].epoch(), 4);
    assert_eq!(headers[11].epoch(), 4);

    // Now verify the entire chain sequentially
    let mut verify_state = create_test_genesis_state();

    for (i, block) in blocks.iter().enumerate() {
        let parent_header = if i == 0 {
            None
        } else {
            Some(headers[i - 1].clone())
        };

        // Verify this block succeeds
        assert_verification_succeeds(
            &mut verify_state,
            block.header(),
            parent_header.clone(),
            block.body(),
        );

        // Verify state is updated correctly after each block
        let expected_slot = i as u64;

        // Calculate expected state epoch based on new structure
        let block_epoch = if i == 0 {
            0 // Genesis is epoch 0
        } else {
            (expected_slot - 1) / BLOCKS_PER_EPOCH + 1
        };

        // Check if this block is terminal
        let is_terminal = if i == 0 {
            true // Genesis is terminal
        } else {
            expected_slot.is_multiple_of(BLOCKS_PER_EPOCH) // Slots 3, 6, 9 are terminal
        };

        // After terminal block, state epoch is incremented.
        let expected_state_epoch = block_epoch + (is_terminal as u64);
        assert_state_updated(&mut verify_state, expected_state_epoch, expected_slot);

        // Special checks for epoch transitions.
        if i > 0 && headers[i].epoch() != headers[i - 1].epoch() {
            // This is an epoch initial block (follows a terminal block)
            assert_eq!(
                headers[i].epoch(),
                headers[i - 1].epoch() + 1,
                "Epoch should increment by exactly 1 at transition"
            );

            // The previous block should have been terminal
            assert!(
                headers[i - 1].is_terminal(),
                "Block {} should be terminal (last block of epoch {})",
                i - 1,
                headers[i - 1].epoch()
            );

            // The epochal state's epoch equals the block's epoch at this point
            assert_eq!(
                verify_state.cur_epoch(),
                headers[i].epoch(),
                "Epochal state should match block epoch after epoch transition"
            );
        }
    }

    // Final verification - ensure the verify_state matches the assembly state
    assert_eq!(
        state.cur_epoch(),
        verify_state.cur_epoch(),
        "Assembly and verification states should have same epoch"
    );
    assert_eq!(
        state.cur_slot(),
        verify_state.cur_slot(),
        "Assembly and verification states should have same slot"
    );

    // Verify state roots match between assembly and verification
    let assembly_state_root = state
        .compute_state_root()
        .expect("Assembly state root should compute");
    let verify_state_root = verify_state
        .compute_state_root()
        .expect("Verify state root should compute");
    assert_eq!(
        assembly_state_root, verify_state_root,
        "State roots should match between assembly and verification"
    );
}
