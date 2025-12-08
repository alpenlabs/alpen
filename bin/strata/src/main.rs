//! Strata client binary entrypoint.

use std::time::Duration;

use strata_common::logging;
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

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    // Validate params, configs and create node context.
    let nodectx = init_node_context(args).map_err(|e| anyhow::anyhow!("{}", e))?;

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

fn do_startup_checks(_ctx: &NodeContext) -> anyhow::Result<()> {
    // TODO: things like if bitcoin client is running or not, db consistency checks and any other
    // checks prior to starting services, GENESIS checks etc.
    Ok(())
}

fn init_logging(rt: &Handle) {
    let mut lconfig = logging::LoggerConfig::with_base_name("strata-client");

    let otlp_url = logging::get_otlp_url_from_env();
    if let Some(url) = &otlp_url {
        lconfig.set_otlp_url(url.clone());
    }

    {
        let _g = rt.enter();
        logging::init(lconfig);
    }

    if let Some(url) = &otlp_url {
        info!(%url, "using OpenTelemetry tracing output");
    }
}
