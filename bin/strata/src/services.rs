//! Service spawning and lifecycle management.

use jsonrpsee::{RpcModule, types::ErrorObjectOwned};
use strata_consensus_logic::sync_manager::{spawn_asm_worker, spawn_csm_listener};

use crate::{context::NodeContext, run_context::RunContext};

/// Just simply starts services. This can later be extended to service registry pattern.
pub(crate) fn start_services(nodectx: NodeContext) -> anyhow::Result<RunContext> {
    // Start Asm worker
    let asm_handle = spawn_asm_worker(
        &nodectx.executor,
        nodectx.executor.handle().clone(),
        nodectx.storage.clone(),
        nodectx.params.clone(),
        nodectx.bitcoin_client.clone(),
    )?;

    // Start Csm worker
    let csm_monitor = spawn_csm_listener(
        &nodectx.executor,
        nodectx.params.clone(),
        nodectx.storage.clone(),
        (*nodectx.status_channel).clone(),
        asm_handle.monitor(),
    )?;

    // TODO: Start other tasks like l1writer, mempool, broadcaster, fcm, btcio reader, etc. all as
    // service, returning the monitors.
    Ok(RunContext {
        config: nodectx.config,
        params: nodectx.params,
        task_manager: nodectx.task_manager,
        executor: nodectx.executor,
        asm_handle,
        csm_monitor,
    })
}

pub(crate) fn start_rpc(runctx: &RunContext) -> anyhow::Result<()> {
    let rpc_host = runctx.config.client.rpc_host.clone();
    let rpc_port = runctx.config.client.rpc_port;
    runctx
        .executor
        .spawn_critical_async("main-rpc", spawn_rpc(rpc_host, rpc_port));
    Ok(())
}

async fn spawn_rpc(rpc_host: String, rpc_port: u16) -> anyhow::Result<()> {
    let mut module = RpcModule::new(());
    let _ = module.register_method("strata_protocolVersion", |_, _, _ctx| {
        Ok::<u32, ErrorObjectOwned>(1)
    });

    let rpc_server = jsonrpsee::server::ServerBuilder::new()
        .build(format!("{rpc_host}:{rpc_port}"))
        .await
        .expect("init: build rpc server");

    let rpc_handle = rpc_server.start(module);

    // wait for rpc to stop
    rpc_handle.stopped().await;

    Ok(())
}
