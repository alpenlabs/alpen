//! Standalone sequencer signer for Strata.
//!
//! Connects to a sequencer node via RPC, fetches signing duties,
//! and submits signatures. Private keys never leave this process.

mod args;
mod config;
mod constants;
mod helpers;

use std::{fs, sync::Arc, time::Duration};

use args::Args;
use config::SignerConfig;
use constants::SHUTDOWN_TIMEOUT_MS;
use helpers::load_seqkey;
use strata_common::ws_client::{ManagedWsClient, WsClientConfig};
use strata_logging::{init_logging_from_config, LoggingInitConfig};
use strata_signer::SignerBuilder;
use strata_tasks::TaskManager;
use tokio::runtime::Builder;
use tracing::info;

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
    init_logging_from_config(LoggingInitConfig {
        service_base_name: "strata-signer",
        service_label: config.logging.service_label.as_deref(),
        otlp_url: config.logging.otlp_url.as_deref(),
        log_dir: config.logging.log_dir.as_ref(),
        log_file_prefix: config.logging.log_file_prefix.as_deref(),
        json_format: config.logging.json_format,
        default_log_prefix: "signer",
        enable_metrics_layer: false,
    });

    // Load sequencer key. Raw bytes are zeroized inside load_seqkey before it returns.
    let (sk, pubkey) = load_seqkey(&config.sequencer_key)?;
    info!(?pubkey, "sequencer key loaded");

    // Set up RPC client.
    let ws_config = WsClientConfig {
        url: config.sequencer_endpoint.clone(),
    };
    let rpc = Arc::new(ManagedWsClient::new_with_default_pool(ws_config));

    info!(sequencer_endpoint = %config.sequencer_endpoint, duty_poll_interval_ms = config.duty_poll_interval, "starting signer");

    // Launch signer service.
    let task_manager = TaskManager::new(handle.clone());
    let executor = task_manager.create_executor();

    let _monitor = handle.block_on(
        SignerBuilder::new(rpc, sk, Duration::from_millis(config.duty_poll_interval))
            .launch(&executor),
    )?;

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)))?;

    Ok(())
}
