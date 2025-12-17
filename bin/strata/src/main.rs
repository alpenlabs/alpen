//! Strata client binary entrypoint.

use std::time::Duration;

use anyhow::{Result, anyhow};
use argh::from_env;
use strata_common::{
    logging,
    logging::{LoggerConfig, get_otlp_url_from_env},
};
use strata_db_types as _;
use tokio::runtime::Handle;
use tracing::info;

use crate::{
    args::Args,
    context::{NodeContext, init_node_context},
    services::{start_rpc, start_services},
};

mod args;
mod config;
mod context;
mod errors;
mod init_db;
mod run_context;
mod services;

fn main() -> Result<()> {
    let args: Args = from_env();

    // Validate params, configs and create node context.
    let nodectx = init_node_context(args)
        .map_err(|e| anyhow!("Failed to initialize node context: {e}"))?;

    init_logging(nodectx.executor.handle());

    do_startup_checks(&nodectx)?;

    // Start services.
    let runctx = start_services(nodectx)?;

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
    // checks prior to starting services, GENESIS checks etc.
    Ok(())
}

fn init_logging(rt: &Handle) {
    // Load environment variables through EnvArgs
    let env_args = args::EnvArgs::from_env();

    // Construct service name with optional label using library utility
    let service_name = logging::format_service_name(
        "strata-client",
        env_args.service_label.as_deref(),
    );

    let mut lconfig = LoggerConfig::new(service_name);

    // Configure OTLP if URL provided via env var
    if let Some(url) = &env_args.otlp_url {
        lconfig.set_otlp_url(url.clone());
    }

    // Configure file logging if log directory provided via env var
    let file_logging_config = env_args.get_file_logging_config();
    if let Some(file_config) = &file_logging_config {
        lconfig = lconfig.with_file_logging(file_config.clone());
    }

    {
        let _g = rt.enter();
        logging::init(lconfig);
    }

    // Log configuration after init
    if let Some(url) = &env_args.otlp_url {
        info!(%url, "using OpenTelemetry tracing output");
    }
    if let Some(file_config) = &file_logging_config {
        info!(
            log_dir = %file_config.directory.display(),
            log_prefix = %file_config.file_name_prefix,
            "file logging enabled"
        );
    }
}
