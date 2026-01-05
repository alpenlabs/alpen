//! Prover client.

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use args::Args;
use bitcoind_async_client::{Auth, Client};
use checkpoint_runner::runner::checkpoint_proof_runner;
use jsonrpsee::http_client::HttpClientBuilder;
use operators::init_operators;
use rpc_server::ProverClientRpc;
use service::{
    new_checkpoint_handler, new_cl_stf_handler, new_evm_ee_stf_handler, ProofContextVariant,
    ProofTask, SledTaskStore,
};
use strata_common::logging;
use strata_db_store_sled::{prover::ProofDBSled, SledDbConfig};
use strata_paas::{ProverServiceBuilder, ProverServiceConfig, ZkVmBackend};
use strata_primitives::proof::ProofZkVm;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder as _;
use strata_tasks::TaskManager;
use tracing::{debug, info};
#[cfg(feature = "sp1")]
use vk_check::{get_checkpoint_groth16_vk, validate_checkpoint_vk};
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

mod args;
mod checkpoint_runner;
mod config;
mod errors;
mod operators;
mod rpc_server;
mod service;

#[cfg(feature = "sp1")]
mod vk_check;

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("FATAL ERROR: {e}");

        return Err(e);
    }

    Ok(())
}

fn main_inner(args: Args) -> anyhow::Result<()> {
    let base_config = if let Some(config_path) = &args.config {
        config::ProverConfig::from_file(config_path)?
    } else {
        config::ProverConfig::default()
    };

    // Initialize logging using common service
    logging::init_logging_from_config(logging::LoggingInitConfig {
        service_base_name: "strata-prover-client",
        service_label: base_config.logging.service_label.as_deref(),
        otlp_url: base_config.logging.otlp_url.as_deref(),
        log_dir: base_config.logging.log_dir.as_ref(),
        log_file_prefix: base_config.logging.log_file_prefix.as_deref(),
        json_format: base_config.logging.json_format,
        default_log_prefix: "alpen",
    });

    // Resolve configuration from TOML file and CLI arguments
    let config = args
        .resolve_config()
        .context("Failed to resolve configuration")?;

    debug!("Running prover client with config {:?}", config);

    let rollup_params = args
        .resolve_and_validate_rollup_params()
        .context("Failed to resolve and validate rollup parameters")?;

    // Validate checkpoint VK matches between params and the elf file.
    #[cfg(feature = "sp1")]
    {
        // The checkpoint elf lazily initializes after this call and later
        // the checkpoint proving task utilizes the same.
        let checkpoint_vk =
            get_checkpoint_groth16_vk().context("Failed to get checkpoint verification key")?;
        let params_vk = rollup_params
            .checkpoint_predicate
            .as_buf_ref()
            .condition()
            .to_vec();
        validate_checkpoint_vk(&checkpoint_vk, &params_vk)
            .context("Checkpoint verification key validation failed")?;
    }

    let el_client = HttpClientBuilder::default()
        .build(config.get_reth_rpc_url())
        .context("Failed to connect to the Ethereum client")?;

    let cl_client = HttpClientBuilder::default()
        .build(config.get_sequencer_rpc_url())
        .context("Failed to connect to the CL Sequencer client")?;

    let auth = Auth::UserPass(
        config.bitcoind_user.clone(),
        config.bitcoind_password.clone(),
    );

    let btc_client = Client::new(
        config.bitcoind_url.clone(),
        auth,
        Some(config.bitcoin_retry_count),
        Some(config.bitcoin_retry_interval),
        None,
    )
    .context("Failed to connect to the Bitcoin client")?;

    // Initialize operators
    let (checkpoint_operator, cl_stf_operator, evm_ee_operator) =
        init_operators(btc_client, el_client, cl_client, rollup_params);

    let sled_db =
        strata_db_store_sled::open_sled_database(&config.datadir, strata_db_store_sled::SLED_NAME)
            .context("Failed to open the Sled database")?;
    let retries = 3;
    let delay_ms = 200;
    let db_config = SledDbConfig::new_with_constant_backoff(retries, delay_ms);
    let db = Arc::new(ProofDBSled::new(sled_db, db_config)?);

    // Create task store for persistence
    let task_store = SledTaskStore::new(db.clone());

    // Create Prover Service configuration
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

    let service_config = ProverServiceConfig::new(worker_counts);

    // Create runtime and task manager
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("prover-rt")
        .build()
        .context("Failed to build runtime")?;
    let task_manager = TaskManager::new(runtime.handle().clone());
    let executor = task_manager.create_executor();

    // Create handlers for each proof type
    let checkpoint_handler = Arc::new(new_checkpoint_handler(
        checkpoint_operator.clone(),
        db.clone(),
        executor.clone(),
    ));

    let cl_stf_handler = Arc::new(new_cl_stf_handler(
        cl_stf_operator.clone(),
        db.clone(),
        executor.clone(),
    ));

    let evm_ee_handler = Arc::new(new_evm_ee_stf_handler(
        evm_ee_operator.clone(),
        db.clone(),
        executor.clone(),
    ));

    // Create and launch Prover Service with handlers
    let builder = ProverServiceBuilder::<ProofTask>::new(service_config)
        .with_task_store(task_store)
        .with_retry_config(strata_paas::RetryConfig::default())
        .with_handler(ProofContextVariant::Checkpoint, checkpoint_handler)
        .with_handler(ProofContextVariant::ClStf, cl_stf_handler)
        .with_handler(ProofContextVariant::EvmEeStf, evm_ee_handler);

    // Launch the service
    let service_handle = runtime
        .block_on(builder.launch(&executor))
        .context("Failed to launch prover service")?;

    debug!("Initialized Prover Service");

    // run the checkpoint runner
    if config.enable_checkpoint_runner {
        let checkpoint_operator_clone = checkpoint_operator.clone();
        let checkpoint_handle = service_handle.clone();
        let checkpoint_poll_interval = config.checkpoint_poll_interval;
        let checkpoint_db = db.clone();
        executor.spawn_critical_async("checkpoint-runner", async move {
            checkpoint_proof_runner(
                checkpoint_operator_clone,
                checkpoint_poll_interval,
                checkpoint_handle,
                checkpoint_db,
            )
            .await;
            Ok(())
        });
        debug!("Spawned checkpoint proof runner");
    }

    let rpc_server = ProverClientRpc::new(
        service_handle.clone(),
        checkpoint_operator,
        cl_stf_operator,
        db,
    );
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
