//! Prover client.

use std::sync::Arc;

use anyhow::Context;
use args::Args;
use bitcoind_async_client::Client;
use checkpoint_runner::runner::checkpoint_proof_runner;
use db::open_sled_database;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::ProofOperator;
use prover_manager::{ProverManager, ProverManagerConfig};
use rpc_server::ProverClientRpc;
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use task_tracker::TaskTracker;
use tokio::{spawn, sync::Mutex};
use tracing::debug;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

mod args;
mod checkpoint_runner;
mod config;
mod db;
mod errors;
mod operators;
mod prover_manager;
mod retry_policy;
mod rpc_server;
mod status;
mod task_tracker;

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
        config.enable_checkpoint_runner,
    ));
    let task_tracker = Arc::new(Mutex::new(TaskTracker::new()));

    let sled_db =
        open_sled_database(&config.datadir).context("Failed to open the Sled database")?;
    let retries = 3;
    let delay_ms = 200;
    let db_config = SledDbConfig::new_with_constant_backoff(retries, delay_ms);
    let db = Arc::new(ProofDBSled::new(sled_db, db_config)?);

    let prover_config = ProverManagerConfig::new(
        config.get_workers(),
        config.polling_interval,
        config.max_retry_counter,
    );
    let manager = ProverManager::new(
        task_tracker.clone(),
        operator.clone(),
        db.clone(),
        prover_config,
    );
    debug!("Initialized Prover Manager");

    // Run prover manager in background
    spawn(async move { manager.process_pending_tasks().await });
    debug!("Spawn process pending tasks");

    // run the checkpoint runner
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

    let rpc_server = ProverClientRpc::new(task_tracker.clone(), operator, db);
    rpc_server
        .start_server(config.get_dev_rpc_url(), config.enable_dev_rpcs)
        .await
        .context("Failed to start the RPC server")?;

    Ok(())
}
