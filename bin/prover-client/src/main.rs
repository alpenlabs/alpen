//! Prover client.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use args::Args;
use bitcoind_async_client::Client;
use checkpoint_runner::runner::checkpoint_proof_runner;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::ProofOperator;
use rpc_server::ProverClientRpc;
use service::{CheckpointInputProvider, ClStfInputProvider, EvmEeInputProvider, ProofStoreService};
use service::{ProofContextVariant, ProofTask};
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
use strata_paas::{PaaSConfig, ProverServiceBuilder, ZkVmBackend};
use strata_primitives::proof::ProofZkVm;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use strata_proofimpl_cl_stf::program::ClStfProgram;
use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
use strata_tasks::TaskManager;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use tracing::{debug, info};
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

mod args;
mod checkpoint_runner;
mod config;
mod errors;
mod host_resolver;
mod operators;
mod rpc_server;
mod service;

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("FATAL ERROR: {e}");

        return Err(e);
    }

    Ok(())
}

fn main_inner(args: Args) -> anyhow::Result<()> {
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

    let sled_db = strata_db_store_sled::open_sled_database(
        &config.datadir,
        strata_db_store_sled::SLED_NAME,
    )
    .context("Failed to open the Sled database")?;
    let retries = 3;
    let delay_ms = 200;
    let db_config = SledDbConfig::new_with_constant_backoff(retries, delay_ms);
    let db = Arc::new(ProofDBSled::new(sled_db, db_config)?);

    // Create PaaS components
    let checkpoint_input = CheckpointInputProvider {
        operator: operator.checkpoint_operator().clone(),
        db: db.clone(),
    };
    let cl_stf_input = ClStfInputProvider {
        operator: operator.cl_stf_operator().clone(),
        db: db.clone(),
    };
    let evm_ee_input = EvmEeInputProvider {
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
            *workers.get(&ProofZkVm::SP1).unwrap_or(&0),
        );
    }
    worker_counts.insert(
        ZkVmBackend::Native,
        *workers.get(&ProofZkVm::Native).unwrap_or(&1),
    );

    let paas_config = PaaSConfig::new(worker_counts);

    // Create runtime and task manager
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("prover-rt")
        .build()
        .context("Failed to build runtime")?;
    let task_manager = TaskManager::new(runtime.handle().clone());
    let executor = task_manager.create_executor();

    // Create and launch PaaS service
    // Register each program type with its handler and host
    let builder = ProverServiceBuilder::<ProofTask>::new(paas_config)
        .with_program::<CheckpointProgram, _, _, _>(
            ProofContextVariant::Checkpoint,
            checkpoint_input,
            proof_store.clone(),
            resolve_host!(host_resolver::sample_checkpoint()),
        )
        .with_program::<ClStfProgram, _, _, _>(
            ProofContextVariant::ClStf,
            cl_stf_input,
            proof_store.clone(),
            resolve_host!(host_resolver::sample_cl_stf()),
        )
        .with_program::<EvmEeProgram, _, _, _>(
            ProofContextVariant::EvmEeStf,
            evm_ee_input,
            proof_store,
            resolve_host!(host_resolver::sample_evm_ee()),
        );

    // Launch the service
    let paas_handle = runtime
        .block_on(builder.launch(&executor))
        .context("Failed to launch prover service")?;

    debug!("Initialized PaaS prover service");

    // run the checkpoint runner
    if config.enable_checkpoint_runner {
        let checkpoint_operator = operator.checkpoint_operator().clone();
        let checkpoint_handle = paas_handle.clone();
        let checkpoint_poll_interval = config.checkpoint_poll_interval;
        let checkpoint_db = db.clone();
        executor.spawn_critical_async("checkpoint-runner", async move {
            checkpoint_proof_runner(
                checkpoint_operator,
                checkpoint_poll_interval,
                checkpoint_handle,
                checkpoint_db,
            )
            .await;
            Ok(())
        });
        debug!("Spawned checkpoint proof runner");
    }

    let rpc_server = ProverClientRpc::new(paas_handle.clone(), operator, db);
    let rpc_url = config.get_dev_rpc_url();
    let enable_dev_rpcs = config.enable_dev_rpcs;
    executor.spawn_critical_async("rpc-server", async move {
        rpc_server
            .start_server(rpc_url, enable_dev_rpcs)
            .await
            .context("Failed to start the RPC server")
    });

    info!("All services started");

    // Monitor tasks and block until shutdown
    task_manager.start_signal_listeners();
    task_manager.monitor(Some(std::time::Duration::from_secs(5)))?;

    info!("Shutting down");
    Ok(())
}
