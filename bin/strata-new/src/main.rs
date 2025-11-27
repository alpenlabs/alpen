use std::time::Duration;

use jsonrpsee::{RpcModule, types::ErrorObjectOwned};
use strata_asm_worker::AsmWorkerHandle;
use strata_common::logging;
use strata_config::{ClientConfig, Config};
use strata_consensus_logic::{asm_worker_context::AsmWorkerCtx, sync_manager::spawn_csm_listener};
use strata_db_types as _;
use tokio::runtime::Handle;
use tracing::info;

use crate::{
    args::Args,
    context::{NodeContext, init_node_context},
    helpers::{get_config, resolve_and_validate_params},
};

mod args;
mod context;
mod errors;
mod helpers;
mod init_db;

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    let config = get_config(args.clone()).expect("config: load error");
    let params =
        resolve_and_validate_params(&args.rollup_params.expect("args: no params path"), &config)
            .unwrap();

    // Initialize runtime, database, etc and wrap them in the node context.
    let nodectx = init_node_context(config, params).unwrap();

    // Initialize logging.
    init_logging(nodectx.executor.handle());

    // Startup checks.
    do_startup_checks(&nodectx);

    // Start services
    //
    // Start ASM
    let asm_handle = spawn_asm_worker(&nodectx).unwrap();

    // Start CSM
    let _csm_monitor = spawn_csm_listener(
        &nodectx.executor,
        nodectx.params.clone(),
        nodectx.storage.clone(),
        (*nodectx.status_channel).clone(),
        asm_handle.monitor(),
    )
    .unwrap();

    // TODO: Start other tasks like l1writer, mempool, broadcaster, fcm, btcio reader, etc. all as
    // service, returning the monitors.

    // Start rpc
    // TODO: pass in service monitors/handlers
    nodectx
        .executor
        .spawn_critical_async("main-rpc", start_rpc(nodectx.config.client.clone()));

    nodectx.task_manager.start_signal_listeners();
    nodectx.task_manager.monitor(Some(Duration::from_secs(5)))?;

    info!("Exiting strata");

    Ok(())
}

/// Server exposing just a method for protocol version.
async fn start_rpc(config: ClientConfig) -> anyhow::Result<()> {
    let mut module = RpcModule::new(());
    let _ = module.register_method("strata_protocolVersion", |_, _, _ctx| {
        Ok::<u32, ErrorObjectOwned>(1)
    });

    let rpc_host = config.rpc_host;
    let rpc_port = config.rpc_port;

    let rpc_server = jsonrpsee::server::ServerBuilder::new()
        .build(format!("{rpc_host}:{rpc_port}"))
        .await
        .expect("init: build rpc server");

    let rpc_handle = rpc_server.start(module);

    // wait for rpc to stop
    rpc_handle.stopped().await;

    Ok(())
}

fn do_startup_checks(_ctx: &NodeContext) {
    // TODO: things like if bitcoin client is running or not, db consistency checks and any other
    // checks prior to starting services, genesis checks etc.
}

fn spawn_asm_worker(nodectx: &NodeContext) -> anyhow::Result<AsmWorkerHandle> {
    let ctx = AsmWorkerCtx::new(
        nodectx.executor.handle().clone(),
        nodectx.bitcoin_client.clone(),
        nodectx.storage.l1().clone(),
        nodectx.storage.asm().clone(),
    );

    let handle = strata_asm_worker::AsmWorkerBuilder::new()
        .with_context(ctx)
        .with_params(nodectx.params.clone())
        .launch(&nodectx.executor)?;

    Ok(handle)
}

/// Sets up the logging system given a handle to a runtime context to possibly
/// start the OTLP output on.
fn init_logging(rt: &Handle) {
    let mut lconfig = logging::LoggerConfig::with_base_name("strata-client");

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
