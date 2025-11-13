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
use paas_integration::{CheckpointFetcher, ClStfFetcher, EvmEeFetcher, ProofStoreService};
use rpc_server::ProverClientRpc;
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
use strata_paas::{PaaSConfig, ProofContextVariant, RegistryProverServiceBuilder, ZkVmBackend};
use strata_primitives::proof::ProofContext;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use strata_proofimpl_cl_stf::program::ClStfProgram;
use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
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
mod host_resolver;
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

    // Create PaaS components using registry-based API
    let checkpoint_fetcher = CheckpointFetcher {
        operator: operator.checkpoint_operator().clone(),
        db: db.clone(),
    };
    let cl_stf_fetcher = ClStfFetcher {
        operator: operator.cl_stf_operator().clone(),
        db: db.clone(),
    };
    let evm_ee_fetcher = EvmEeFetcher {
        operator: operator.evm_ee_operator().clone(),
        db: db.clone(),
    };
    let proof_store = ProofStoreService::new(db.clone(), operator.checkpoint_operator().clone());

    // Create PaaS configuration
    let mut worker_counts = HashMap::new();
    let workers = config.get_workers();

    // Configure workers for each backend
    #[cfg(feature = "sp1")]
    {
        worker_counts.insert(
            ZkVmBackend::SP1,
            *workers
                .get(&strata_primitives::proof::ProofZkVm::SP1)
                .unwrap_or(&0),
        );
    }
    worker_counts.insert(
        ZkVmBackend::Native,
        *workers
            .get(&strata_primitives::proof::ProofZkVm::Native)
            .unwrap_or(&1),
    );

    let paas_config = PaaSConfig::new(worker_counts);

    // Create task manager and executor
    let task_manager = TaskManager::new(tokio::runtime::Handle::current());
    let executor = task_manager.create_executor();

    // Create and launch PaaS service with registry-based API
    // Register each program type with its handler and host
    let builder = RegistryProverServiceBuilder::<ProofContext>::new(paas_config)
        .register::<CheckpointProgram, _, _, _>(
            ProofContextVariant::Checkpoint,
            checkpoint_fetcher,
            proof_store.clone(),
            resolve_host!(host_resolver::sample_checkpoint()),
        )
        .register::<ClStfProgram, _, _, _>(
            ProofContextVariant::ClStf,
            cl_stf_fetcher,
            proof_store.clone(),
            resolve_host!(host_resolver::sample_cl_stf()),
        )
        .register::<EvmEeProgram, _, _, _>(
            ProofContextVariant::EvmEeStf,
            evm_ee_fetcher,
            proof_store,
            resolve_host!(host_resolver::sample_evm_ee()),
        );

    // Launch the service
    let paas_handle = builder
        .launch(&executor)
        .await
        .context("Failed to launch prover service")?;

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
