//! Strata client binary entrypoint.

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use argh::from_env;
use strata_common::logging;
use strata_consensus_logic::sync_manager::spawn_asm_worker_with_ctx;
use strata_db_types as _;
use strata_node_context::NodeContext;
use tokio::runtime::{self, Handle};
use tracing::info;

use crate::{
    args::Args,
    context::{check_and_init_genesis, init_node_context},
    errors::InitError,
    services::{start_rpc, start_strata_services},
};

mod args;
mod config;
mod context;
mod errors;
mod genesis;
mod init_db;
mod rpc;
mod run_context;
mod services;

fn main() -> Result<()> {
    let args: Args = from_env();

    // Load config early to initialize logging with config settings
    let config = context::load_config_early(&args)
        .map_err(|e| anyhow!("Failed to load configuration: {e}"))?;
    // Init runtime
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .map_err(InitError::RuntimeBuild)?;

    // Validate params, configs and create node context.
    let nodectx = init_node_context(args, config.clone(), rt.handle().clone())
        .map_err(|e| anyhow!("Failed to initialize node context: {e}"))?;

    init_logging(nodectx.executor().handle(), &config);

    do_startup_checks(&nodectx)?;

    // start Asm Service for genesis manifest, or maybe we can just start bitcoin reader service
    // that feeds L1BlockCommitment to Asm
    let asm_handle = Arc::new(spawn_asm_worker_with_ctx(&nodectx)?);

    // Check and do genesis if not yet
    check_and_init_genesis(nodectx.storage().as_ref(), nodectx.params().as_ref())?;

    // Start services.
    let runctx = start_strata_services(nodectx, asm_handle)?;

    // Start RPC.
    start_rpc(&runctx)?;

    // Monitor tasks.
    runctx.task_manager.start_signal_listeners();
    runctx.task_manager.monitor(Some(Duration::from_secs(5)))?;

    info!("Exiting strata");
    Ok(())
}

fn do_startup_checks(_ctx: &NodeContext) -> Result<()> {
    // TODO: things like if bitcoin client is running or not, db consistency checks and any other
    // checks prior to starting services, etc.

    Ok(())
}

fn init_logging(rt: &Handle, config: &strata_config::Config) {
    // Need to set the runtime context for async OTLP setup
    let _g = rt.enter();
    logging::init_logging_from_config(logging::LoggingInitConfig {
        service_base_name: "strata-client",
        service_label: config.logging.service_label.as_deref(),
        otlp_url: config.logging.otlp_url.as_deref(),
        log_dir: config.logging.log_dir.as_ref(),
        log_file_prefix: config.logging.log_file_prefix.as_deref(),
        json_format: config.logging.json_format,
        default_log_prefix: "alpen",
    });
}
