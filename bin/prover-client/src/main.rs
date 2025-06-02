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

    debug!("Running prover client with args {:?}", args);

    let rollup_params = args
        .resolve_and_validate_rollup_params()
        .context("Failed to resolve and validate rollup parameters")?;

    let el_client = HttpClientBuilder::default()
        .build(args.get_reth_rpc_url())
        .context("Failed to connect to the Ethereum client")?;

    let cl_client = HttpClientBuilder::default()
        .build(args.get_sequencer_rpc_url())
        .context("Failed to connect to the CL Sequencer client")?;

    let btc_client = Client::new(
        args.get_btc_rpc_url(),
        args.bitcoind_user.clone(),
        args.bitcoind_password.clone(),
        args.bitcoin_retry_count,
        args.bitcoin_retry_interval,
    )
    .context("Failed to connect to the Bitcoin client")?;

    let operator = Arc::new(ProofOperator::init(
        btc_client,
        el_client,
        cl_client,
        rollup_params,
        args.enable_checkpoint_runner,
    ));
    let task_tracker = Arc::new(Mutex::new(TaskTracker::new()));

    let rbdb =
        open_rocksdb_database(&args.datadir).context("Failed to open the RocksDB database")?;
    let db_ops = DbOpsConfig { retry_count: 3 };
    let db = Arc::new(ProofDb::new(rbdb, db_ops));

    let manager = ProverManager::new(
        task_tracker.clone(),
        operator.clone(),
        db.clone(),
        args.get_workers(),
        args.polling_interval,
    );
    debug!("Initialized Prover Manager");

    // Run prover manager in background
    spawn(async move { manager.process_pending_tasks().await });
    debug!("Spawn process pending tasks");

    // run the checkpoint runner
    if args.enable_checkpoint_runner {
        let checkpoint_operator = operator.checkpoint_operator().clone();
        let task_tracker_for_checkpoint = task_tracker.clone();
        let db_for_checkpoint = db.clone();

        spawn(async move {
            checkpoint_proof_runner(
                checkpoint_operator,
                task_tracker_for_checkpoint,
                db_for_checkpoint,
            )
            .await
        });
    }

    // Run the RPC server on dev mode only
    if args.enable_dev_rpcs {
        let rpc_url = args.get_dev_rpc_url();
        run_rpc_server(
            task_tracker.clone(),
            operator.clone(),
            db.clone(),
            rpc_url,
            args.enable_dev_rpcs,
        )
        .await
        .context("Failed to run the prover client RPC server")?;
    }

    Ok(())
}

async fn run_rpc_server(
    task_tracker: Arc<Mutex<TaskTracker>>,
    operator: Arc<ProofOperator>,
    db: Arc<ProofDb>,
    rpc_url: String,
    enable_dev_rpc: bool,
) -> anyhow::Result<()> {
    let rpc_impl = ProverClientRpc::new(task_tracker, operator, db);
    rpc_server::start(&rpc_impl, rpc_url, enable_dev_rpc).await?;
    anyhow::Ok(())
}
