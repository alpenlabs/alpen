//! Prover client.

use std::sync::Arc;

use anyhow::Context;
use args::Args;
use bitcoind_async_client::Client;
use checkpoint_runner::runner::checkpoint_proof_runner;
use db::open_sled_database;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::ProofOperator;
use paas_adapter::ProofOperatorAdapter;
use rpc_server::ProverClientRpc;
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
use strata_paas::{FeatureConfig, PaaSConfig, ProverBuilder, RetryConfig, WorkerConfig};
#[cfg(feature = "sp1")]
use strata_primitives::proof::ProofZkVm;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use strata_tasks::TaskManager;
use task_tracker_adapter::TaskTrackerAdapter;
use tokio::{runtime::Handle, spawn, sync::Mutex};
use tracing::debug;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

mod args;
mod checkpoint_runner;
mod config;
mod db;
mod errors;
mod operators;
mod paas_adapter;
mod prover_manager;
mod retry_policy;
mod rpc_server;
mod status;
mod task_tracker;
mod task_tracker_adapter;

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

    // Open database
    let sled_db =
        open_sled_database(&config.datadir).context("Failed to open the Sled database")?;
    let retries = 3;
    let delay_ms = 200;
    let db_config = SledDbConfig::new_with_constant_backoff(retries, delay_ms);
    let db = Arc::new(ProofDBSled::new(sled_db, db_config)?);

    // Initialize proof operator
    let operator = Arc::new(ProofOperator::init(
        btc_client,
        el_client,
        cl_client,
        rollup_params,
        config.enable_checkpoint_runner,
    ));

    // Configure PaaS worker settings
    let mut worker_counts = std::collections::HashMap::new();
    #[cfg(feature = "sp1")]
    {
        worker_counts.insert(
            ProofZkVm::SP1,
            config
                .get_workers()
                .get(&ProofZkVm::SP1)
                .copied()
                .unwrap_or(1),
        );
    }
    #[cfg(not(feature = "sp1"))]
    {
        use strata_primitives::proof::ProofZkVm;
        worker_counts.insert(
            ProofZkVm::Native,
            config
                .get_workers()
                .get(&ProofZkVm::Native)
                .copied()
                .unwrap_or(1),
        );
    }

    let worker_config = WorkerConfig {
        worker_count: worker_counts,
        polling_interval_ms: config.polling_interval,
    };

    let retry_config = RetryConfig {
        max_retries: config.max_retry_counter as u32,
        base_delay_secs: 1,
        max_delay_secs: 3600,
        multiplier: 1.5,
    };

    let paas_config = PaaSConfig {
        workers: worker_config,
        retry: retry_config,
        features: FeatureConfig {
            enable_checkpoint_runner: config.enable_checkpoint_runner,
        },
    };

    // Create task manager for service lifecycle
    let task_manager = TaskManager::new(Handle::current());
    let executor = task_manager.create_executor();

    // Launch ProverService using builder and get handle
    let prover_handle = ProverBuilder::new()
        .with_config(paas_config.clone())
        .with_database(db.clone())
        .launch(&executor)
        .await
        .context("Failed to launch ProverService")?;
    debug!("Launched ProverService");

    // Wrap prover_handle in Arc for sharing
    let prover_handle_arc = Arc::new(prover_handle);

    // Create TaskTracker adapter for operator compatibility
    let task_tracker_adapter = TaskTrackerAdapter::new(prover_handle_arc.clone());
    let task_tracker = Arc::new(Mutex::new(task_tracker_adapter));

    // Create ProofOperator adapter and spawn worker pool
    let proof_operator_adapter = Arc::new(ProofOperatorAdapter::from_arc(operator.clone()));
    let worker_pool = strata_paas::WorkerPool::new(
        prover_handle_arc.clone(),
        proof_operator_adapter,
        db.clone(),
        paas_config,
    );

    spawn(async move {
        worker_pool.run().await;
        debug!("Worker pool completed");
    });
    debug!("Spawned worker pool");

    // TODO: Migrate checkpoint runner to use ProverHandle
    // Run checkpoint runner if enabled
    if config.enable_checkpoint_runner {
        let checkpoint_operator = operator.checkpoint_operator().clone();
        let checkpoint_task_tracker = task_tracker.clone();
        let checkpoint_poll_interval = config.checkpoint_poll_interval;
        let checkpoint_db = db.clone();
        spawn(async move {
            checkpoint_proof_runner(
                checkpoint_operator,
                checkpoint_poll_interval,
                checkpoint_task_tracker,
                checkpoint_db,
            )
            .await;
        });
        debug!("Spawned checkpoint proof runner");
    }

    // Start RPC server with adapter
    let rpc_server = ProverClientRpc::new(task_tracker.clone(), operator, db);
    rpc_server
        .start_server(config.get_dev_rpc_url(), config.enable_dev_rpcs)
        .await
        .context("Failed to start the RPC server")?;

    Ok(())
}
