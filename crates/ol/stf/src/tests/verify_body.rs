//! Body commitment and round-trip verification tests for the OL STF implementation.

use strata_identifiers::Buf32;
use strata_ledger_types::IStateAccessor;

use crate::{assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*};

#[test]
fn test_verify_valid_block_succeeds() {
    // This test verifies that a properly assembled block passes verification
    let mut state = make_genesis_state();

    // Assemble genesis block (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("Genesis block assembly should succeed");

    // Reset state for verification (verification should start from same initial state)
    let mut verify_state = make_genesis_state();

    // Verify the block - this should succeed
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
}

#[test]
fn test_assemble_then_verify_roundtrip() {
    // This test verifies the full round-trip: assemble blocks then verify them
    let mut state = make_genesis_state();

    // Assemble genesis block (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("test: Genesis block assembly should succeed");

    // Assemble block 1 (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("test: Block 1 assembly should succeed");

    // Assemble block 2 (still epoch 1)
    let block2_info = BlockInfo::new(1002000, 2, 1);
    let block2 = execute_block(
        &mut state,
        &block2_info,
        Some(block1.header()),
        BlockComponents::new_empty(),
    )
    .expect("test: Block 2 assembly should succeed");

    // Now verify the entire chain
    let mut verify_state = make_genesis_state();

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
fn test_multi_block_chain_verification() {
    // Test verifying a longer chain across epoch boundaries
    let mut state = make_genesis_state();
    const SLOTS_PER_EPOCH: u64 = 10;

    // Build a chain of blocks
    let mut blocks = Vec::new();
    let mut headers = Vec::new();

    // Build 15 blocks (crossing multiple epochs)
    for i in 0..15 {
        let slot = i as u64;
        // With genesis as terminal: epoch 0 is just slot 0, then epochs are 10 slots each
        let epoch = if i == 0 {
            0 // Genesis is epoch 0
        } else {
            ((slot - 1) / SLOTS_PER_EPOCH + 1) as u32 // Slots 1-10 are epoch 1, 11-20 are epoch 2, etc.
        };
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = if i == 0 {
            BlockInfo::new_genesis(timestamp)
        } else {
            BlockInfo::new(timestamp, slot, epoch)
        };

        let parent_header = if i == 0 { None } else { Some(&headers[i - 1]) };

        // Check if this should be a terminal block
        // Genesis (slot 0) is terminal, then slots 10, 20, etc.
        let is_terminal = if i == 0 {
            true // Genesis is always terminal
        } else {
            slot.is_multiple_of(SLOTS_PER_EPOCH) // Slots 10, 20, etc. are terminal
        };

        let components = if i == 0 {
            build_terminal_genesis_components()
        } else if is_terminal {
            build_terminal_block_components(state.last_l1_height() + 1)
        } else {
            BlockComponents::new_empty()
        };

        let Ok(block) = execute_block(&mut state, &block_info, parent_header, components) else {
            panic!("test: block {i} assembly should succeed");
        };

        headers.push(block.header().clone());
        blocks.push(block);
    }

    // Now verify the entire chain
    let mut verify_state = make_genesis_state();

    for (i, block) in blocks.iter().enumerate() {
        let parent_header = if i == 0 {
            None
        } else {
            Some(headers[i - 1].clone())
        };

        assert_verification_succeeds(
            &mut verify_state,
            block.header(),
            parent_header,
            block.body(),
        );
    }

    // Verify final state matches
    // With genesis as terminal: slot 14 is in epoch 2
    assert_state_position(&verify_state, 2, 14);
}

#[test]
fn test_verify_block_with_transactions() {
    // Test that blocks with transactions can be verified
    let mut state = make_genesis_state();

    // Create a transaction
    let target = make_account_id(1);
    let gam_tx = make_gam_tx(target);

    // Assemble terminal genesis with transaction.
    let genesis_components = build_terminal_tx_components(vec![gam_tx]);

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_components)
        .expect("Genesis with tx should assemble");

    // Verify the block
    let mut verify_state = make_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Verify transaction was included
    assert_eq!(
        genesis
            .body()
            .tx_segment()
            .expect("genesis should have tx_segment")
            .txs()
            .len(),
        1
    );
}

#[test]
fn test_verify_rejects_mismatched_state_root() {
    // Test that verification fails when state root doesn't match computed
    let mut state = make_genesis_state();

    // Assemble a normal genesis block (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("Genesis assembly should succeed");

    // Tamper with the state root in the header
    let wrong_root = Buf32::from([99u8; 32]);
    let tampered_header = tamper_state_root(genesis.header(), wrong_root);

    let mut positive_verify_state = make_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );

    // Verification should fail because computed state root won't match header
    let mut verify_state = make_genesis_state();
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        None,
        genesis.body(),
        |e| matches!(e, ExecError::ChainIntegrity),
    );
}

#[test]
fn test_verify_rejects_mismatched_logs_root() {
    // Test that verification fails when logs root doesn't match computed
    let mut state = make_genesis_state();

    // Create a block with a transaction (which will generate logs)
    let target = make_account_id(1);
    let gam_tx = make_gam_tx(target);

    // Create terminal genesis with transaction.
    let genesis_components = build_terminal_tx_components(vec![gam_tx]);

    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_components)
        .expect("Genesis assembly should succeed");

    // Tamper with the logs root
    let wrong_root = Buf32::from([88u8; 32]);
    let tampered_header = tamper_logs_root(genesis.header(), wrong_root);

    let mut positive_verify_state = make_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );

    // Verification should fail because computed logs root won't match header
    let mut verify_state = make_genesis_state();
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        None,
        genesis.body(),
        |e| matches!(e, ExecError::ChainIntegrity),
    );
}

#[test]
fn test_verify_empty_block_logs_root() {
    // Test that empty blocks should have zero logs root
    let mut state = make_genesis_state();

    // Assemble genesis block (terminal but with no transactions)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("Genesis assembly should succeed");

    // Verify that empty blocks have zero logs root
    assert_eq!(
        *genesis.header().logs_root(),
        Buf32::zero(),
        "Empty block should have zero logs root"
    );

    // Verify the block succeeds
    let mut verify_state = make_genesis_state();

    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
}

#[test]
fn test_verify_rejects_mismatched_body_root() {
    // Test that verification fails when body root doesn't match body hash.
    let mut state = make_genesis_state();

    // Assemble genesis first.
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("Genesis assembly should succeed");

    // Assemble a non-genesis block with a transaction.
    let target = make_account_id(1);
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![make_gam_tx(target)]),
    )
    .expect("Block 1 assembly should succeed");

    // Tamper with the body root.
    let wrong_root = Buf32::from([77u8; 32]);
    let tampered_header = tamper_body_root(block1.header(), wrong_root);

    // Positive control: untampered block verifies.
    let mut positive_verify_state = make_genesis_state();
    assert_verification_succeeds(
        &mut positive_verify_state,
        genesis.header(),
        None,
        genesis.body(),
    );
    assert_verification_succeeds(
        &mut positive_verify_state,
        block1.header(),
        Some(genesis.header().clone()),
        block1.body(),
    );

    let mut verify_state = make_genesis_state();
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(genesis.header().clone()),
        block1.body(),
        |e| matches!(e, ExecError::BlockStructureMismatch),
    );
}

#[test]
fn test_verify_state_root_changes_with_state() {
    // Test that state root properly reflects state changes
    let mut state = make_genesis_state();

    // Execute genesis (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        build_terminal_genesis_components(),
    )
    .expect("Genesis should execute");

    // Execute block 1 (will change slot in state, epoch 1)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 should execute");

    // State roots should be different
    assert_ne!(
        genesis.header().state_root(),
        block1.header().state_root(),
        "State root should change when state changes"
    );

    // Now verify both blocks
    let mut verify_state = make_genesis_state();

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
