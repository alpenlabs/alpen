//! Core ASM worker integration tests
//!
//! Tests the ASM worker's ability to process Bitcoin blocks and maintain state.

// Suppress unused crate warnings - these dependencies are used by other test files
use anyhow as _;
use bitcoin::Network;
use bitcoind_async_client as _;
use borsh as _;
use common::{asm::TestAsmWorkerContext, harness::create_test_harness};
use corepc_node as _;
use integration_tests::common;
use rand as _;
use strata_asm_common as _;
use strata_asm_manifest_types as _;
use strata_asm_proto_administration as _;
use strata_asm_txs_admin as _;
use strata_btc_types as _;
use strata_crypto as _;
use strata_merkle as _;
use strata_params as _;
use strata_primitives::L1BlockId;
use strata_state as _;
use strata_tasks as _;
use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};
use strata_test_utils_l2 as _;

#[cfg(test)]
mod tests {
    use strata_asm_worker::WorkerContext;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_asm_worker_context_creation() {
        let (_bitcoind, client) = get_bitcoind_and_client();
        let context = TestAsmWorkerContext::new(client);

        // Verify it returns Regtest network
        assert_eq!(context.get_network().unwrap(), Network::Regtest);

        // Verify initial state is empty
        assert!(context.get_latest_asm_state().unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_blocks_from_regtest() {
        let (bitcoind, client) = get_bitcoind_and_client();
        let context = TestAsmWorkerContext::new(client);

        // Mine 5 blocks
        let block_hashes = mine_blocks(&bitcoind, context.client.as_ref(), 5, None)
            .await
            .expect("Failed to mine blocks");

        println!("Mined {} blocks", block_hashes.len());

        // Fetch each block through the context
        for (i, block_hash) in block_hashes.iter().enumerate() {
            let block_id = L1BlockId::from(*block_hash);
            let block = context
                .get_l1_block(&block_id)
                .expect("Failed to get block");
            println!("Fetched block {}: {} txs", i + 1, block.txdata.len());
        }

        // Verify blocks are now cached
        assert_eq!(context.block_cache.lock().unwrap().len(), 5);
        println!("All {} blocks cached", 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_and_cache_block() {
        let (bitcoind, client) = get_bitcoind_and_client();
        let context = TestAsmWorkerContext::new(client.clone());

        // Mine a block
        let block_hashes = mine_blocks(&bitcoind, &client, 1, None)
            .await
            .expect("Failed to mine block");

        let block_hash = block_hashes[0];

        // Fetch and cache
        let block = context
            .fetch_and_cache_block(block_hash)
            .await
            .expect("Failed to fetch block");

        println!("Fetched block with {} transactions", block.txdata.len());

        // Verify it's cached
        let block_id = L1BlockId::from(block_hash);
        assert!(context.block_cache.lock().unwrap().contains_key(&block_id));

        // Fetch again - should come from cache
        let cached_block = context
            .get_l1_block(&block_id)
            .expect("Failed to get cached block");
        assert_eq!(block.block_hash(), cached_block.block_hash());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_asm_worker_basic_processing() {
        // Create test harness with ASM worker service running
        let harness = create_test_harness()
            .await
            .expect("Failed to create test harness");

        println!(
            "Harness created with genesis at height {}",
            harness.genesis_height
        );

        // Mine and submit a block - ASM worker processes it automatically
        let block_hash = harness
            .mine_and_submit_block(None)
            .await
            .expect("Failed to mine and submit block");

        println!("Mined and submitted block: {}", block_hash);

        // Wait for ASM worker to process
        harness.wait_for_processing().await;

        // Verify chain tip advanced
        let tip_height = harness
            .get_chain_tip()
            .await
            .expect("Failed to get chain tip");
        assert_eq!(tip_height, harness.genesis_height + 1);

        println!("Block processed successfully! Chain tip: {}", tip_height);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_with_harness() {
        // Create test harness with ASM worker service running
        let harness = create_test_harness()
            .await
            .expect("Failed to create test harness");

        println!("Harness created successfully");

        // Mine and submit blocks - ASM worker processes them automatically
        let block_hashes = harness
            .mine_and_submit_blocks(3)
            .await
            .expect("Failed to mine blocks");

        println!("Mined and submitted {} blocks", block_hashes.len());

        // Wait for processing
        harness.wait_for_processing().await;

        // Verify chain tip advanced
        let tip_height = harness
            .get_chain_tip()
            .await
            .expect("Failed to get chain tip");
        assert_eq!(tip_height, harness.genesis_height + 3);

        println!("Test with harness completed successfully!");
    }

    // TODO: Add test_empty_blocks_state_progression
    // TODO: Add test_state_persistence
}
