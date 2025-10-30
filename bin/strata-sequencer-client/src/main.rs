//! Strata sequencer client
//!
//! Responsible for signing blocks and checkpoints
//! Note: currently this only functions as a 'signer' and does not perform any
//! transaction sequencing or block building duties.

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
use tokio::{runtime::Handle, sync::mpsc};
use tracing::info;

const SHUTDOWN_TIMEOUT_MS: u64 = 5000;

fn main() -> Result<()> {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("FATAL ERROR: {e}");

        return Err(e);
    }

    Ok(())
}

fn main_inner(args: Args) -> Result<()> {
    // Start runtime for async IO tasks.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .expect("init: build rt");
    let handle = runtime.handle();

    // Init the logging before we do anything else.
    init_logging(handle);

    let config = get_config(args.clone())?;
    let idata = load_seqkey(&config.sequencer_key)?;

    let task_manager = TaskManager::new(handle.clone());
    let executor = task_manager.create_executor();

    let ws_url = config.ws_url();
    info!("connecting to strata client at {}", ws_url);

    let rpc = Arc::new(rpc_client(&ws_url));

    let (duty_tx, duty_rx) = mpsc::channel(64);

    executor.spawn_critical_async(
        "duty-fetcher",
        duty_fetcher_worker(rpc.clone(), duty_tx, config.duty_poll_interval),
    );
    executor.spawn_critical_async(
        "duty-runner",
        duty_executor_worker(rpc, duty_rx, handle.clone(), idata, config.epoch_gas_limit),
    );

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_millis(SHUTDOWN_TIMEOUT_MS)))?;

    Ok(())
}

fn get_config(args: Args) -> Result<Config> {
    Config::from_args(&args).map_err(AppError::InvalidArgs)
}

/// Sets up the logging system given a handle to a runtime context to possibly
/// start the OTLP output on.
fn init_logging(rt: &Handle) {
    let mut lconfig = logging::LoggerConfig::with_base_name("strata-sequencer");

    // Set the OpenTelemetry URL if set.
    let otlp_url = logging::get_otlp_url_from_env();
    if let Some(url) = &otlp_url {
        lconfig.set_otlp_url(url.clone());
    }

    {
        // Need to set the runtime context because of nonsense.
        let _g = rt.enter();
        logging::init(lconfig);
    }

    // Have to log this after we start the logging formally.
    if let Some(url) = &otlp_url {
        info!(%url, "using OpenTelemetry tracing output");
    }
}
