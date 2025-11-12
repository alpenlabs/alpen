//! Prover client.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use args::Args;
use bitcoind_async_client::Client;
use checkpoint_runner::runner::checkpoint_proof_runner;
use db::open_sled_database;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::ProofOperator;
use paas_integration::{DynamicHostProver, ProverInputFetcher, ProverProofStore};
use rpc_server::ProverClientRpc;
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
use strata_paas::{PaaSConfig, ProverService, ProverServiceState, ZkVmBackend};
use strata_service::ServiceBuilder;
use strata_tasks::TaskManager;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use tokio::spawn;
use tracing::debug;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

mod args;
mod checkpoint_runner;
mod config;
mod db;
mod errors;
mod operators;
mod paas_integration;
mod rpc_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args).await {
        eprintln!("FATAL ERROR: {e}");

        return Err(e);
    }

    Ok(())
}

async fn main_inner(args: Args) -> anyhow::Result<()> {
    logging::init(logging::LoggerConfig::with_base_name(
        "strata-prover-client",
    ));

    // Resolve configuration from TOML file and CLI arguments
    let config = args
        .resolve_config()
        .context("Failed to resolve configuration")?;

    debug!("Running prover client with config {:?}", config);

    let rollup_params = args
        .resolve_and_validate_rollup_params()
        .context("Failed to resolve and validate rollup parameters")?;

    let el_client = HttpClientBuilder::default()
        .build(config.get_reth_rpc_url())
        .context("Failed to connect to the Ethereum client")?;

    let cl_client = HttpClientBuilder::default()
        .build(config.get_sequencer_rpc_url())
        .context("Failed to connect to the CL Sequencer client")?;

    let btc_client = Client::new(
        config.bitcoind_url.clone(),
        config.bitcoind_user.clone(),
        config.bitcoind_password.clone(),
        Some(config.bitcoin_retry_count),
        Some(config.bitcoin_retry_interval),
    )
    .context("Failed to connect to the Bitcoin client")?;

    let operator = Arc::new(ProofOperator::init(
        btc_client,
        el_client,
        cl_client,
        rollup_params,
    ));

    let sled_db =
        open_sled_database(&config.datadir).context("Failed to open the Sled database")?;
    let retries = 3;
    let delay_ms = 200;
    let db_config = SledDbConfig::new_with_constant_backoff(retries, delay_ms);
    let db = Arc::new(ProofDBSled::new(sled_db, db_config)?);

    // Create PaaS components
    let input_fetcher = Arc::new(ProverInputFetcher::new(
        operator.evm_ee_operator().clone(),
        operator.cl_stf_operator().clone(),
        operator.checkpoint_operator().clone(),
        db.clone(),
    ));
    let proof_store = Arc::new(ProverProofStore::new(db.clone()));
    let dynamic_prover = Arc::new(DynamicHostProver::new(input_fetcher, proof_store));

    // Create PaaS configuration
    let mut worker_counts = HashMap::new();
    let workers = config.get_workers();

    // Configure workers for each backend
    #[cfg(feature = "sp1")]
    {
        worker_counts.insert(ZkVmBackend::SP1, *workers.get(&strata_primitives::proof::ProofZkVm::SP1).unwrap_or(&0));
    }
    worker_counts.insert(ZkVmBackend::Native, *workers.get(&strata_primitives::proof::ProofZkVm::Native).unwrap_or(&1));

    let paas_config = PaaSConfig::new(worker_counts);

    // Create task manager and executor
    let task_manager = TaskManager::new(tokio::runtime::Handle::current());
    let executor = task_manager.create_executor();

    // Create and launch PaaS service
    let service_state = ProverServiceState::new(dynamic_prover, paas_config);
    let mut service_builder = ServiceBuilder::<ProverService<DynamicHostProver>, _>::new()
        .with_state(service_state);

    let prover_handle = service_builder.create_command_handle(100);
    let prover_monitor = service_builder
        .launch_async("prover", &executor)
        .await
        .context("Failed to launch prover service")?;

    let paas_handle = strata_paas::ProverHandle::<strata_primitives::proof::ProofContext>::new(prover_handle, prover_monitor);

    debug!("Initialized PaaS prover service");

    // run the checkpoint runner
    if config.enable_checkpoint_runner {
        let checkpoint_operator = operator.checkpoint_operator().clone();
        let checkpoint_handle = paas_handle.clone();
        let checkpoint_poll_interval = config.checkpoint_poll_interval;
        let checkpoint_db = db.clone();
        spawn(async move {
            checkpoint_proof_runner(
                checkpoint_operator,
                checkpoint_poll_interval,
                checkpoint_handle,
                checkpoint_db,
            )
            .await;
        });
        debug!("Spawned checkpoint proof runner");
    }

    let rpc_server = ProverClientRpc::new(paas_handle.clone(), operator, db);
    rpc_server
        .start_server(config.get_dev_rpc_url(), config.enable_dev_rpcs)
        .await
        .context("Failed to start the RPC server")?;

    Ok(())
}
