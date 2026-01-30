//! Strata sequencer signer.

mod args;
mod config;
mod duty_executor;
mod duty_fetcher;
mod errors;
mod helpers;
mod rpc_client;

use std::{sync::Arc, time::Duration};

use args::Args;
use config::Config;
use duty_executor::duty_executor_worker;
use duty_fetcher::duty_fetcher_worker;
use errors::{AppError, Result};
use helpers::load_seqkey;
use rpc_client::rpc_client;
use strata_common::logging;
use strata_tasks::TaskManager;
use tokio::{
    runtime::{Builder, Handle},
    sync::mpsc,
};
use tracing::info;

const SHUTDOWN_TIMEOUT_MS: u64 = 5000;

fn main() -> Result<()> {
    let args: Args = argh::from_env();
    main_inner(args)
}

fn main_inner(args: Args) -> Result<()> {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .expect("init: build rt");
    let handle = runtime.handle();

    let config = Config::from_args(&args).map_err(AppError::InvalidArgs)?;

    init_logging(handle, &config);

    let idata = load_seqkey(&config.sequencer_key)?;
    let task_manager = TaskManager::new(handle.clone());
    let executor = task_manager.create_executor();

    let ws_url = config.ws_url();
    info!(%ws_url, "connecting to strata client");

    let rpc = Arc::new(rpc_client(&ws_url));
    let (duty_tx, duty_rx) = mpsc::channel(64);

    executor.spawn_critical_async(
        "duty-fetcher",
        duty_fetcher_worker(rpc.clone(), duty_tx, config.duty_poll_interval),
    );
    executor.spawn_critical_async(
        "duty-runner",
        duty_executor_worker(rpc, duty_rx, handle.clone(), idata),
    );

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)))?;

    Ok(())
}

/// Sets up the logging system given a handle to a runtime context to possibly
/// start the OTLP output on.
fn init_logging(rt: &Handle, config: &Config) {
    let _g = rt.enter();
    logging::init_logging_from_config(logging::LoggingInitConfig {
        service_base_name: "strata-sequencer",
        service_label: config.logging.service_label.as_deref(),
        otlp_url: config.logging.otlp_url.as_deref(),
        log_dir: config.logging.log_dir.as_ref(),
        log_file_prefix: config.logging.log_file_prefix.as_deref(),
        json_format: config.logging.json_format,
        default_log_prefix: "alpen",
    });
}
