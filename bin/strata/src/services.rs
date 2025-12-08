//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use strata_consensus_logic::sync_manager::{spawn_asm_worker, spawn_csm_listener};
use strata_rpc_api_new::OLClientRpcServer;

use crate::{context::NodeContext, rpc::OLRpcServer, run_context::RunContext};

/// Just simply starts services. This can later be extended to service registry pattern.
pub(crate) fn start_services(nodectx: NodeContext) -> Result<RunContext> {
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
        runtime: nodectx.runtime,
        config: nodectx.config,
        params: nodectx.params,
        task_manager: nodectx.task_manager,
        executor: nodectx.executor,
        asm_handle,
        csm_monitor,
        storage: nodectx.storage,
        status_channel: nodectx.status_channel,
    })
}

/// Starts the RPC server.
pub(crate) fn start_rpc(runctx: &RunContext) -> Result<()> {
    let rpc_host = runctx.config.client.rpc_host.clone();
    let rpc_port = runctx.config.client.rpc_port;
    let storage = runctx.storage.clone();
    let status_channel = runctx.status_channel.clone();
    runctx.executor.spawn_critical_async(
        "main-rpc",
        spawn_rpc(rpc_host, rpc_port, storage, status_channel),
    );
    Ok(())
}

/// Spawns the RPC server.
async fn spawn_rpc(
    rpc_host: String,
    rpc_port: u16,
    storage: Arc<strata_storage::NodeStorage>,
    status_channel: Arc<strata_status::StatusChannel>,
) -> Result<()> {
    let mut module = RpcModule::new(());

    // Register existing protocol version method
    let _ = module.register_method("strata_protocolVersion", |_, _, _ctx| {
        Ok::<u32, ErrorObjectOwned>(1)
    });

    // Create and register OL RPC server
    let ol_rpc_server = OLRpcServer::new(storage, status_channel);
    let ol_module = OLClientRpcServer::into_rpc(ol_rpc_server);
    module
        .merge(ol_module)
        .map_err(|e| anyhow!("Failed to merge OL RPC module: {}", e))?;

    let rpc_server = ServerBuilder::new()
        .build(format!("{rpc_host}:{rpc_port}"))
        .await
        .map_err(|e| anyhow!("Failed to build RPC server on {rpc_host}:{rpc_port}: {e}"))?;

    let rpc_handle = rpc_server.start(module);

    // wait for rpc to stop
    rpc_handle.stopped().await;

    Ok(())
}
