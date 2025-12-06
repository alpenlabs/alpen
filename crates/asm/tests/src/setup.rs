use std::sync::Arc;

use bitcoin::Network;
use bitcoind_async_client::{traits::Reader, Client};
use corepc_node::Node;
use strata_asm_worker::AsmWorkerServiceState;
use strata_params::Params;
use strata_test_utils_l2::gen_params;

use crate::{
    context::MockWorkerContext,
    utils::{get_bitcoind_and_client, get_genesis_l1_view, mine_blocks},
};

#[allow(dead_code)]
pub struct TestEnv {
    pub _node: Node, // Keep node alive
    pub client: Arc<Client>,
    pub context: MockWorkerContext,
    pub service_state: AsmWorkerServiceState<MockWorkerContext>,
    pub params: Arc<Params>,
}

pub async fn setup_env() -> TestEnv {
    // 1. Setup Bitcoin Regtest
    let (node, client) = get_bitcoind_and_client();
    let client = Arc::new(client);

    // Mine some initial blocks to have funds and chain height.
    let _ = mine_blocks(&node, &client, 101, None)
        .await
        .expect("Failed to mine initial blocks");

    // Pick the current tip as our "genesis" for the ASM.
    let tip_hash = client.get_block_hash(101).await.unwrap();

    // 2. Setup Params
    let mut params = gen_params();
    params.rollup.network = Network::Regtest;

    // Sync parameters with the actual bitcoind state
    let genesis_view = get_genesis_l1_view(&client, &tip_hash)
        .await
        .expect("Failed to fetch genesis view");
    params.rollup.genesis_l1_view = genesis_view;

    let params = Arc::new(params);

    // 3. Setup Worker Context
    let context = MockWorkerContext::new();

    // 4. Initialize Service State
    let mut service_state = AsmWorkerServiceState::new(context.clone(), params.clone());

    // Initialize: this should create genesis state based on our `genesis_l1_view`
    service_state
        .load_latest_or_create_genesis()
        .expect("Failed to load/create genesis state");

    assert!(service_state.initialized);
    assert!(service_state.anchor.is_some());

    println!("Service initialized with genesis at height 101");

    TestEnv {
        _node: node,
        client,
        context,
        service_state,
        params,
    }
}
