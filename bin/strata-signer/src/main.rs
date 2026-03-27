//! Standalone sequencer signer for Strata.
//!
//! Connects to a sequencer node via RPC, fetches signing duties,
//! and submits signatures. Private keys never leave this process.

mod args;
mod config;
mod constants;
mod duty_executor;
mod duty_fetcher;
mod helpers;

use std::{fs, sync::Arc, time::Duration};

use args::Args;
use config::SignerConfig;
use constants::SHUTDOWN_TIMEOUT_MS;
use duty_executor::duty_executor_worker;
use duty_fetcher::duty_fetcher_worker;
use helpers::load_seqkey;
use strata_common::{
    logging,
    ws_client::{ManagedWsClient, WsClientConfig},
};
use strata_tasks::TaskManager;
use tokio::{runtime::Builder, sync::mpsc};
use tracing::info;
use zeroize::Zeroize;

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    // Load config from TOML file.
    let config_str = fs::read_to_string(&args.config)?;
    let config: SignerConfig = toml::from_str(&config_str)?;

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name("signer-rt")
        .build()
        .expect("failed to build tokio runtime");

    let handle = runtime.handle();

    // Init logging. Need runtime context for async OTLP setup.
    let _g = handle.enter();
    logging::init_logging_from_config(logging::LoggingInitConfig {
        service_base_name: "strata-signer",
        service_label: config.logging.service_label.as_deref(),
        otlp_url: config.logging.otlp_url.as_deref(),
        log_dir: config.logging.log_dir.as_ref(),
        log_file_prefix: config.logging.log_file_prefix.as_deref(),
        json_format: config.logging.json_format,
        default_log_prefix: "signer",
    });

    // Load sequencer key.
    let mut seq_key = load_seqkey(&config.sequencer_key)?;
    info!(pubkey = ?seq_key.pk, "sequencer key loaded");

    // Set up RPC client.
    let ws_config = WsClientConfig {
        url: config.sequencer_endpoint.clone(),
    };
    let rpc = Arc::new(ManagedWsClient::new_with_default_pool(ws_config));

    info!(sequencer_endpoint = %config.sequencer_endpoint, duty_poll_interval_ms = config.duty_poll_interval, "starting signer");

    // Duty channel.
    let (duty_tx, duty_rx) = mpsc::channel(64);

    // Spawn workers.
    let task_manager = TaskManager::new(handle.clone());
    let executor = task_manager.create_executor();

    executor.spawn_critical_async(
        "duty-fetcher",
        duty_fetcher_worker(rpc.clone(), duty_tx, config.duty_poll_interval),
    );
    executor.spawn_critical_async(
        "duty-executor",
        duty_executor_worker(rpc, duty_rx, seq_key.sk),
    );

    // Zeroize the key now that it's been moved into the worker.
    seq_key.zeroize();

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)))?;

    Ok(())
}
