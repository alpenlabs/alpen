//! Test environment for ASM integration tests.
//!
//! Provides a unified test environment with Bitcoin regtest and all ASM subprotocols.

use std::{num::NonZero, sync::Arc};

use bitcoin::{Block, BlockHash, Network, absolute::Height, block::Header, secp256k1::Secp256k1};
use bitcoind_async_client::{
    Client,
    traits::{Reader, Wallet},
};
use corepc_node::Node;
use strata_asm_proto_administration::{AdministrationSubprotoParams, AdministrationSubprotoState};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1State};
use strata_asm_proto_checkpoint::{CheckpointConfig, CheckpointState};
use strata_asm_worker::AsmWorkerServiceState;
use strata_crypto::threshold_signature::{CompressedPublicKey, ThresholdConfig};
use strata_identifiers::{CredRule, L1BlockCommitment, L1BlockId};
use strata_predicate::PredicateKey;
use strata_primitives::l1::GenesisL1View;
use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};
use strata_test_utils_l2::gen_params;

use crate::{test_data::CheckpointGenerator, worker_context::MockWorkerContext};

/// Test environment for ASM integration tests with Bitcoin regtest.
///
/// Provides access to:
/// - Bitcoin regtest node and RPC client
/// - ASM worker service state
/// - All subprotocol states (checkpoint, bridge, admin)
/// - Test fixtures for generating test data
#[derive(Debug)]
pub struct AsmTestEnv {
    /// Bitcoin node (kept alive for test duration).
    pub node: Node,
    /// Bitcoin RPC client.
    pub client: Arc<Client>,
    /// Checkpoint generator for creating test data.
    pub checkpoint_generator: CheckpointGenerator,
    /// ASM worker service state.
    pub service_state: AsmWorkerServiceState<MockWorkerContext>,
    /// Genesis L1 view used for initialization.
    pub genesis_view: GenesisL1View,
    /// Checkpoint subprotocol state.
    pub checkpoint_state: CheckpointState,
    /// Bridge V1 subprotocol state.
    pub bridge_state: BridgeV1State,
    /// Administration subprotocol state.
    pub admin_state: AdministrationSubprotoState,
}

impl AsmTestEnv {
    /// Create a new test environment with Bitcoin regtest and all ASM subprotocols.
    pub async fn new() -> Self {
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

        // Create checkpoint generator with sequencer keypair
        let checkpoint_generator = CheckpointGenerator::new();

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
            sequencer_cred: CredRule::SchnorrKey(checkpoint_generator.sequencer.public_key),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_block: genesis_view.blk,
        };
        let checkpoint_state = CheckpointState::new(&checkpoint_config);

        // Create bridge V1 subprotocol state
        let bridge_config = BridgeV1Config {
            operators: vec![],
            denomination: 1_000_000.into(), // 0.01 BTC
            assignment_duration: 144,       // ~1 day in blocks
            operator_fee: 1_000.into(),     // 1000 sats
        };
        let bridge_state = BridgeV1State::new(&bridge_config);

        // Create administration subprotocol state
        let admin_config = create_test_admin_config();
        let admin_state = AdministrationSubprotoState::new(&admin_config);

        Self {
            node,
            client,
            checkpoint_generator,
            service_state,
            genesis_view,
            checkpoint_state,
            bridge_state,
            admin_state,
        }
    }

    /// Mine blocks to confirm a transaction.
    pub async fn mine_and_confirm(&self, num_blocks: u64) -> anyhow::Result<Vec<BlockHash>> {
        let address = self.client.get_new_address().await?;
        mine_blocks(&self.node, &self.client, num_blocks as usize, Some(address)).await
    }

    /// Get a block by hash.
    pub async fn get_block(&self, hash: &BlockHash) -> anyhow::Result<Block> {
        Ok(self.client.get_block(hash).await?)
    }
}

/// Create a test admin config with default threshold configurations.
fn create_test_admin_config() -> AdministrationSubprotoParams {
    use bitcoin::secp256k1::{PublicKey, SecretKey};
    use rand::rngs::OsRng;

    let secp = Secp256k1::new();

    // Create admin keys (3-of-2 multisig)
    let admin_sks: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
    let admin_pks: Vec<CompressedPublicKey> = admin_sks
        .iter()
        .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
        .collect();
    let strata_administrator =
        ThresholdConfig::try_new(admin_pks, NonZero::new(2).unwrap()).unwrap();

    // Create sequencer manager keys (3-of-2 multisig)
    let seq_sks: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
    let seq_pks: Vec<CompressedPublicKey> = seq_sks
        .iter()
        .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
        .collect();
    let strata_sequencer_manager =
        ThresholdConfig::try_new(seq_pks, NonZero::new(2).unwrap()).unwrap();

    AdministrationSubprotoParams {
        strata_administrator,
        strata_sequencer_manager,
        confirmation_depth: 6, // 6 blocks for tests
    }
}

/// Helper to construct `GenesisL1View` from a block hash.
pub async fn get_genesis_l1_view(
    client: &Client,
    hash: &BlockHash,
) -> anyhow::Result<GenesisL1View> {
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
