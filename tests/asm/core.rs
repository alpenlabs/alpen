//! Core ASM worker integration tests
//!
//! Tests the ASM worker's ability to process Bitcoin blocks and maintain state.

#![allow(
    unused_crate_dependencies,
    reason = "test dependencies shared across test suite"
)]

use bitcoin::Network;
use harness::{test_harness::create_test_harness, worker_context::TestAsmWorkerContext};
use integration_tests::harness;
use strata_asm_worker::WorkerContext;
use strata_primitives::L1BlockId;
use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};

// ============================================================================
// Worker Context
// ============================================================================

/// Verifies worker context initializes with correct defaults.
#[tokio::test(flavor = "multi_thread")]
async fn test_worker_context_initialization() {
    let (_bitcoind, client) = get_bitcoind_and_client();
    let context = TestAsmWorkerContext::new(client);

    assert_eq!(context.get_network().unwrap(), Network::Regtest);
    assert!(context.get_latest_asm_state().unwrap().is_none());
}

/// Verifies blocks are fetched from regtest and cached.
#[tokio::test(flavor = "multi_thread")]
async fn test_block_fetching_and_caching() {
    let (bitcoind, client) = get_bitcoind_and_client();
    let context = TestAsmWorkerContext::new(client);

    // Mine 5 blocks
    let block_hashes = mine_blocks(&bitcoind, context.client.as_ref(), 5, None)
        .await
        .expect("Failed to mine blocks");

    // Fetch each block through the context
    for block_hash in block_hashes.iter() {
        let block_id = L1BlockId::from(*block_hash);
        context
            .get_l1_block(&block_id)
            .expect("Failed to get block");
    }

    // Verify blocks are cached
    assert_eq!(context.block_cache.lock().unwrap().len(), 5);

    // Fetch again - should come from cache
    let block_id = L1BlockId::from(block_hashes[0]);
    let block = context
        .get_l1_block(&block_id)
        .expect("Failed to get cached block");
    assert_eq!(block.block_hash(), block_hashes[0]);
}

// ============================================================================
// Block Processing
// ============================================================================

/// Verifies ASM worker processes a single mined block.
#[tokio::test(flavor = "multi_thread")]
async fn test_single_block_processing() {
    let harness = create_test_harness()
        .await
        .expect("Failed to create test harness");

    harness
        .mine_block(None)
        .await
        .expect("Failed to mine block");

    let tip_height = harness
        .get_chain_tip()
        .await
        .expect("Failed to get chain tip");
    assert_eq!(tip_height, harness.genesis_height + 1);
}

/// Verifies ASM worker processes multiple mined blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_block_processing() {
    let harness = create_test_harness()
        .await
        .expect("Failed to create test harness");

    let block_hashes = harness.mine_blocks(3).await.expect("Failed to mine blocks");
    assert_eq!(block_hashes.len(), 3);

    let tip_height = harness
        .get_chain_tip()
        .await
        .expect("Failed to get chain tip");
    assert_eq!(tip_height, harness.genesis_height + 3);
}
