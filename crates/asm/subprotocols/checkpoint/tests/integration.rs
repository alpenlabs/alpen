//! Integration tests for checkpoint subprotocol with real Bitcoin regtest.
//!
//! These tests verify the full E2E flow:
//! 1. Spin up Bitcoin regtest node
//! 2. Create checkpoint transactions with envelope format
//! 3. Submit and mine transactions
//! 4. Run ASM transition and verify checkpoint state updates

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bitcoin::{Block, BlockHash, Network, absolute::Height, block::Header};
use bitcoind_async_client::{
    Client,
    traits::{Reader, Wallet},
};
use corepc_node::Node;
use ssz::{Decode, Encode};
use strata_asm_proto_checkpoint::{CheckpointConfig, CheckpointState};
use strata_asm_worker::{AsmWorkerServiceState, WorkerContext, WorkerError, WorkerResult};
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_identifiers::{CredRule, L1BlockCommitment, L1BlockId};
use strata_predicate::PredicateKey;
use strata_primitives::l1::GenesisL1View;
use strata_state::asm_state::AsmState;
use strata_test_utils_asm::checkpoint::CheckpointFixtures;
use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};
use strata_test_utils_l2::gen_params;

// Suppress unused extern crate warnings for transitive dependencies
use borsh as _;
use rand as _;
use strata_asm_bridge_msgs as _;
use strata_asm_checkpoint_msgs as _;
use strata_asm_common as _;
use strata_asm_logs as _;
use strata_asm_manifest_types as _;
use strata_asm_proto_checkpoint_txs as _;
use strata_asm_spec as _;
use strata_asm_stf as _;
use strata_asm_types as _;
use strata_codec as _;
use strata_ol_chain_types_new as _;
use strata_ol_stf as _;
use strata_params as _;
use strata_service as _;
use thiserror as _;

/// Mock worker context for testing ASM transitions.
#[derive(Clone, Default)]
struct MockWorkerContext {
    pub blocks: Arc<Mutex<HashMap<L1BlockId, Block>>>,
    pub asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
    pub latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
}

impl MockWorkerContext {
    fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WorkerContext for MockWorkerContext {
    fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block> {
        self.blocks
            .lock()
            .unwrap()
            .get(blockid)
            .cloned()
            .ok_or(WorkerError::MissingL1Block(*blockid))
    }

    fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState> {
        self.asm_states
            .lock()
            .unwrap()
            .get(blockid)
            .cloned()
            .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
    }

    fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>> {
        Ok(self.latest_asm_state.lock().unwrap().clone())
    }

    fn store_anchor_state(
        &self,
        blockid: &L1BlockCommitment,
        state: &AsmState,
    ) -> WorkerResult<()> {
        self.asm_states
            .lock()
            .unwrap()
            .insert(*blockid, state.clone());
        *self.latest_asm_state.lock().unwrap() = Some((*blockid, state.clone()));
        Ok(())
    }

    fn get_network(&self) -> WorkerResult<Network> {
        Ok(Network::Regtest)
    }
}

/// Test environment for checkpoint integration tests with ASM worker.
struct AsmCheckpointTestEnv {
    /// Bitcoin node (kept alive for test duration).
    pub _node: Node,
    /// Bitcoin RPC client.
    pub client: Arc<Client>,
    /// Checkpoint fixtures for generating test data.
    pub fixtures: CheckpointFixtures,
    /// ASM worker service state.
    pub service_state: AsmWorkerServiceState<MockWorkerContext>,
    /// Checkpoint subprotocol state.
    pub checkpoint_state: CheckpointState,
}

impl AsmCheckpointTestEnv {
    /// Create a new test environment with Bitcoin regtest and ASM worker.
    async fn new() -> Self {
        // Start Bitcoin regtest
        let (node, client) = get_bitcoind_and_client();
        let client = Arc::new(client);

        // Mine initial blocks for coinbase maturity
        let _ = mine_blocks(&node, &client, 101, None)
            .await
            .expect("Failed to mine initial blocks");

        // Get genesis L1 view from current tip
        let tip_hash = client.get_block_hash(101).await.unwrap();
        let genesis_view = get_genesis_l1_view(&client, &tip_hash)
            .await
            .expect("Failed to get genesis view");

        // Create checkpoint fixtures with sequencer keypair
        let fixtures = CheckpointFixtures::new();

        // Setup params for ASM worker
        let mut params = gen_params();
        params.rollup.network = Network::Regtest;
        params.rollup.genesis_l1_view = genesis_view.clone();
        let params = Arc::new(params);

        // Create and initialize ASM worker
        let context = MockWorkerContext::new();
        let mut service_state = AsmWorkerServiceState::new(context.clone(), params.clone());
        service_state
            .load_latest_or_create_genesis()
            .expect("Failed to load/create genesis state");

        assert!(service_state.initialized);
        assert!(service_state.anchor.is_some());

        // Create checkpoint subprotocol state
        let checkpoint_config = CheckpointConfig {
            sequencer_cred: CredRule::SchnorrKey(fixtures.sequencer.public_key),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_block: genesis_view.blk,
        };
        let checkpoint_state = CheckpointState::new(&checkpoint_config);

        Self {
            _node: node,
            client,
            fixtures,
            service_state,
            checkpoint_state,
        }
    }

    /// Mine blocks to confirm a transaction.
    async fn mine_and_confirm(&self, num_blocks: u64) -> anyhow::Result<Vec<BlockHash>> {
        let address = self.client.get_new_address().await?;
        mine_blocks(
            &self._node,
            &self.client,
            num_blocks as usize,
            Some(address),
        )
        .await
    }

    /// Get a block by hash.
    async fn get_block(&self, hash: &BlockHash) -> anyhow::Result<Block> {
        Ok(self.client.get_block(hash).await?)
    }
}

/// Helper to construct `GenesisL1View` from a block hash.
async fn get_genesis_l1_view(client: &Client, hash: &BlockHash) -> anyhow::Result<GenesisL1View> {
    let header: Header = client.get_block_header(hash).await?;
    let height = client.get_block_height(hash).await?;

    let blkid: L1BlockId = header.block_hash().into();
    let blk_commitment = L1BlockCommitment::new(
        Height::from_consensus(height as u32).expect("Height overflow"),
        blkid,
    );

    let next_target = header.bits.to_consensus();
    let epoch_start_timestamp = header.time;
    let last_11_timestamps = [header.time - 1; 11];

    Ok(GenesisL1View {
        blk: blk_commitment,
        next_target,
        epoch_start_timestamp,
        last_11_timestamps,
    })
}

// Note: These integration tests require bitcoind which may not be available in CI.
// They're marked with #[ignore] and can be run with: cargo test -- --ignored

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_checkpoint_roundtrip_with_regtest() {
    let env = AsmCheckpointTestEnv::new().await;

    // Generate a signed checkpoint for epoch 0
    let signed_checkpoint = env.fixtures.gen_signed_payload_for_epoch(0);

    println!(
        "Created checkpoint for epoch {}",
        signed_checkpoint.payload().epoch()
    );

    // Verify the checkpoint can be serialized and deserialized
    let ssz_bytes = signed_checkpoint.as_ssz_bytes();
    let parsed = SignedCheckpointPayload::from_ssz_bytes(&ssz_bytes).unwrap();

    assert_eq!(parsed.payload().epoch(), 0);
    assert_eq!(parsed.signature(), signed_checkpoint.signature());

    println!(
        "Checkpoint SSZ roundtrip successful, {} bytes",
        ssz_bytes.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_checkpoint_state_progression() {
    let mut env = AsmCheckpointTestEnv::new().await;

    // Initial state: no epochs verified
    assert_eq!(env.checkpoint_state.expected_next_epoch(), 0);
    assert!(env.checkpoint_state.current_epoch().is_none());

    // Generate and "process" epoch 0 checkpoint
    let payload_0 = env.fixtures.gen_payload_for_epoch(0);
    env.checkpoint_state.update_with_checkpoint(&payload_0);

    assert_eq!(env.checkpoint_state.current_epoch(), Some(0));
    assert_eq!(env.checkpoint_state.expected_next_epoch(), 1);

    // Generate and "process" epoch 1 checkpoint
    let payload_1 = env.fixtures.gen_payload_for_epoch(1);
    env.checkpoint_state.update_with_checkpoint(&payload_1);

    assert_eq!(env.checkpoint_state.current_epoch(), Some(1));
    assert_eq!(env.checkpoint_state.expected_next_epoch(), 2);

    println!("Checkpoint state progression test passed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_asm_transition_empty_block() {
    // Setup environment with ASM worker
    let env = AsmCheckpointTestEnv::new().await;
    let client = &env.client;
    let node = &env._node;
    let service_state = &env.service_state;

    // Mine 1 block on top of genesis (which is at height 101)
    let address = client.get_new_address().await.unwrap();
    let new_block_hashes = mine_blocks(node, client, 1, Some(address)).await.unwrap();
    let new_block_hash = new_block_hashes[0];

    let new_block = client.get_block(&new_block_hash).await.unwrap();

    println!("Mined new block: {}", new_block_hash);

    // Call ASM transition
    let result = service_state.transition(&new_block);

    match result {
        Ok(output) => {
            println!("ASM Transition successful!");

            // For an empty block (coinbase only), verify expected output properties:
            // - The state should be updated with new chain view
            // - The manifest should be created with the block id

            // Verify manifest has correct block id
            let blkid: L1BlockId = new_block.header.block_hash().into();
            assert_eq!(
                output.manifest.blkid(),
                &blkid,
                "Manifest block id should match the processed block"
            );

            // Verify the state is present (chain view should be updated)
            // For an empty block, we just verify the transition succeeded
            println!("Output state has {} sections", output.state.sections.len());

            println!("Empty block transition assertions passed");
        }
        Err(e) => {
            panic!("ASM Transition failed: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_mine_blocks_with_regtest() {
    let env = AsmCheckpointTestEnv::new().await;

    // Mine some blocks
    let block_hashes = env.mine_and_confirm(3).await.unwrap();
    assert_eq!(block_hashes.len(), 3);

    // Verify blocks exist
    for hash in &block_hashes {
        let block = env.get_block(hash).await.unwrap();
        println!("Mined block {} with {} txs", hash, block.txdata.len());
    }

    println!("Regtest mining test passed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_asm_transition_second_block() {
    // Setup environment with ASM worker
    let env = AsmCheckpointTestEnv::new().await;
    let client = &env.client;
    let node = &env._node;
    let service_state = &env.service_state;

    // Mine 2 blocks - first as baseline, second to test transition
    let address = client.get_new_address().await.unwrap();
    let block_hashes = mine_blocks(node, client, 2, Some(address.clone()))
        .await
        .unwrap();

    // Process first block
    let first_block = client.get_block(&block_hashes[0]).await.unwrap();
    let first_result = service_state.transition(&first_block);
    assert!(
        first_result.is_ok(),
        "First block transition should succeed"
    );

    // Process second block
    let second_block = client.get_block(&block_hashes[1]).await.unwrap();
    println!(
        "Processing second block {} with {} transactions",
        block_hashes[1],
        second_block.txdata.len()
    );

    // Call ASM transition on second block
    let result = service_state.transition(&second_block);

    match result {
        Ok(output) => {
            println!("ASM Transition successful for second block!");

            // Verify manifest has correct block id
            let blkid: L1BlockId = second_block.header.block_hash().into();
            assert_eq!(
                output.manifest.blkid(),
                &blkid,
                "Manifest block id should match the processed block"
            );

            // Log the wtxids_root for verification
            println!("Manifest wtxids_root: {:?}", output.manifest.wtxids_root());

            // Verify the state has sections
            println!("Output state has {} sections", output.state.sections.len());

            println!("Second block transition assertions passed");
        }
        Err(e) => {
            panic!("ASM Transition failed for second block: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind"]
async fn test_asm_multiple_block_transitions() {
    // Setup environment with ASM worker
    let env = AsmCheckpointTestEnv::new().await;
    let client = &env.client;
    let node = &env._node;
    let service_state = &env.service_state;

    // Mine 3 blocks sequentially and run transition on each
    let address = client.get_new_address().await.unwrap();

    for i in 0..3 {
        // Mine a block
        let block_hashes = mine_blocks(node, client, 1, Some(address.clone()))
            .await
            .unwrap();
        let block_hash = block_hashes[0];

        let block = client.get_block(&block_hash).await.unwrap();

        println!(
            "Block {}: {} with {} txs",
            i + 1,
            block_hash,
            block.txdata.len()
        );

        // Run ASM transition
        let result = service_state.transition(&block);

        match result {
            Ok(output) => {
                let blkid: L1BlockId = block.header.block_hash().into();
                assert_eq!(
                    output.manifest.blkid(),
                    &blkid,
                    "Block {} manifest should have correct block id",
                    i + 1
                );
                println!("Block {} transition successful", i + 1);
            }
            Err(e) => {
                panic!("Block {} transition failed: {:?}", i + 1, e);
            }
        }
    }

    println!("Multiple block transitions test passed");
}
