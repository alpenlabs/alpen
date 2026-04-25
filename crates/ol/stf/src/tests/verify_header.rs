//! Header continuity tamper tests.

use strata_identifiers::Buf32;
use strata_ol_chain_types_new::OLBlockId;

use crate::{assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*};

#[test]
fn test_verify_rejects_wrong_parent_blkid() {
    // Test that verification fails when parent block ID doesn't match
    let mut state = create_test_genesis_state();

    // Assemble genesis and block 1
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Create a tampered header with wrong parent ID
    let wrong_parent_id = OLBlockId::from(Buf32::from([42u8; 32]));
    let tampered_header = tamper_parent_blkid(block1.header(), wrong_parent_id);

    // Verification should fail
    let mut verify_state = create_test_genesis_state();

    // First verify genesis succeeds
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Then verify block 1 with wrong parent ID fails
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(genesis.header().clone()),
        block1.body(),
        |e| matches!(e, ExecError::BlockParentMismatch),
    );
}

#[test]
fn test_verify_rejects_epoch_skip() {
    // Test that verification fails when epoch increases by more than 1
    let mut state = create_test_genesis_state();

    // Assemble genesis (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    // Assemble block 1 normally (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Create a tampered header with epoch 2 (skipping epoch 1)
    let tampered_header = tamper_epoch(block1.header(), 2);

    // Verification should fail
    let mut verify_state = create_test_genesis_state();

    // First verify genesis
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Then verify block with skipped epoch fails
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(genesis.header().clone()),
        block1.body(),
        |e| matches!(e, ExecError::SkipEpochs(_, _)),
    );
}

#[test]
fn test_verify_rejects_slot_skip() {
    // Test that verification fails when slot doesn't increment by exactly 1
    let mut state = create_test_genesis_state();

    // Assemble genesis (terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    // Create block 1 (epoch 1 since genesis was terminal)
    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Tamper with slot
    let tampered_header = tamper_slot(block1.header(), 3);

    // Verification should fail
    let mut verify_state = create_test_genesis_state();

    // First verify genesis
    assert_verification_succeeds(&mut verify_state, genesis.header(), None, genesis.body());

    // Then verify block with skipped slot fails
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(genesis.header().clone()),
        block1.body(),
        |e| matches!(e, ExecError::SkipTooManySlots(_, _)),
    );
}

#[test]
fn test_verify_rejects_slot_backwards() {
    // Test that verification fails when slot goes backwards
    let mut state = create_test_genesis_state();

    // Assemble genesis and block 1
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("Genesis assembly should succeed");

    let block1_info = BlockInfo::new(1001000, 1, 1);
    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Assemble block 2 normally
    let block2_info = BlockInfo::new(1002000, 2, 1);
    let block2 = execute_block(
        &mut state,
        &block2_info,
        Some(block1.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 2 assembly should succeed");

    // Tamper with block 2 to have slot 1 (going backwards from 2 to 1, same as block1)
    let tampered_header = tamper_slot(block2.header(), 1);

    // Verification should fail
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

    // Then verify block 2 with backwards slot fails
    assert_verification_fails_with(
        &mut verify_state,
        &tampered_header,
        Some(block1.header().clone()),
        block2.body(),
        |e| matches!(e, ExecError::SkipTooManySlots(_, _)), /* This will trigger because it's
                                                             * not exactly +1 */
    );
}

#[test]
fn test_verify_rejects_nongenesis_without_parent() {
    // Test that non-genesis blocks must have a parent header
    let mut state = create_test_genesis_state();

    // Create a non-genesis block at slot 1
    let block1_info = BlockInfo::new(1001000, 1, 0);

    // We need to create a fake genesis to use as parent for assembly
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )
    .expect("Genesis assembly should succeed");

    let block1 = execute_block(
        &mut state,
        &block1_info,
        Some(genesis.header()),
        BlockComponents::new_empty(),
    )
    .expect("Block 1 assembly should succeed");

    // Try to verify block 1 without providing parent header
    let mut verify_state = create_test_genesis_state();

    assert_verification_fails_with(
        &mut verify_state,
        block1.header(),
        None, // No parent provided for non-genesis block
        block1.body(),
        |e| matches!(e, ExecError::GenesisCoordsNonzero),
    );
}

#[test]
fn test_verify_rejects_genesis_with_nonnull_parent() {
    // Test that genesis blocks must have null parent
    let mut state = create_test_genesis_state();

    // Assemble a normal genesis block
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(
        &mut state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )
    .expect("Genesis assembly should succeed");

    // Create a tampered genesis with non-null parent
    let fake_parent_id = OLBlockId::from(Buf32::from([42u8; 32]));
    let tampered_genesis = tamper_parent_blkid(genesis.header(), fake_parent_id);

    // Try to verify tampered genesis
    let mut verify_state = create_test_genesis_state();

    assert_verification_fails_with(
        &mut verify_state,
        &tampered_genesis,
        None,
        genesis.body(),
        |e| matches!(e, ExecError::GenesisParentNonnull),
    );
}
