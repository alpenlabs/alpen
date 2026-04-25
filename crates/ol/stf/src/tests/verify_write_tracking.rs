//! Write-tracking verification tests for the OL STF implementation.
//! These tests verify blocks through the `WriteTrackingState + IndexerState`
//! composition used by `chain-worker-new`.

use strata_ledger_types::IStateAccessor;
use strata_ol_state_support_types::{IndexerState, WriteTrackingState};

use crate::{
    assembly::BlockComponents, context::BlockInfo, test_utils::*, verification::verify_block,
};

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
