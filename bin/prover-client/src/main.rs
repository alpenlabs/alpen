//! Prover client.

use std::sync::Arc;

use anyhow::Context;
use args::Args;
use bitcoind_async_client::Client;
use checkpoint_runner::runner::checkpoint_proof_runner;
use db::open_rocksdb_database;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::ProofOperator;
use prover_manager::ProverManager;
use rpc_server::ProverClientRpc;
use strata_common::logging;
#[cfg(feature = "risc0-builder")]
use strata_risc0_guest_builder as _;
use strata_rocksdb::{prover::db::ProofDb, DbOpsConfig};
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use task_tracker::TaskTracker;
use tokio::{spawn, sync::Mutex};
use tracing::debug;
#[cfg(feature = "risc0")]
use zkaleido_risc0_host as _;
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
        config.get_btc_rpc_url(),
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

    let rbdb =
        open_rocksdb_database(&config.datadir).context("Failed to open the RocksDB database")?;
    let db_ops = DbOpsConfig { retry_count: 3 };
    let db = Arc::new(ProofDb::new(rbdb, db_ops));

    let manager = ProverManager::new(
        task_tracker.clone(),
        operator.clone(),
        db.clone(),
        config.get_workers(),
        config.polling_interval,
        config.max_retry_counter,
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
