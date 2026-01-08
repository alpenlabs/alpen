//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use strata_consensus_logic::sync_manager::{spawn_asm_worker, spawn_csm_listener};
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};
use strata_rpc_api_new::OLClientRpcServer;

use crate::{context::NodeContext, rpc::OLRpcServer, run_context::RunContext};

/// Dependencies needed by the RPC server.
/// Grouped to reduce parameter count when spawning the RPC task.
struct RpcDeps {
    rpc_host: String,
    rpc_port: u16,
    storage: Arc<strata_storage::NodeStorage>,
    status_channel: Arc<strata_status::StatusChannel>,
    mempool_handle: MempoolHandle,
}

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

    // Start mempool service
    let mempool_handle = start_mempool(&nodectx)?;

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
        mempool_handle,
    })
}

/// Starts the mempool service.
fn start_mempool(nodectx: &NodeContext) -> Result<MempoolHandle> {
    let config = OLMempoolConfig::default();

    // Get current chain tip from status channel
    let current_tip = nodectx
        .status_channel
        .get_chain_sync_status()
        .map(|status| status.tip)
        .ok_or_else(|| anyhow!("Chain sync status not available, cannot start mempool"))?;

    let storage = nodectx.storage.clone();
    let status_channel = (*nodectx.status_channel).clone();
    let executor = nodectx.executor.clone();

    // block_on is required because start_services is synchronous but we need
    // to initialize the mempool which requires async operations. The mempool
    // handle must be available before RunContext is constructed.
    nodectx.runtime.block_on(async {
        MempoolBuilder::new(config, storage, status_channel, current_tip)
            .launch(&executor)
            .await
    })
}

/// Starts the RPC server.
pub(crate) fn start_rpc(runctx: &RunContext) -> Result<()> {
    // Bundle RPC dependencies from context for the async task
    let deps = RpcDeps {
        rpc_host: runctx.config.client.rpc_host.clone(),
        rpc_port: runctx.config.client.rpc_port,
        storage: runctx.storage.clone(),
        status_channel: runctx.status_channel.clone(),
        mempool_handle: runctx.mempool_handle.clone(),
    };

    runctx
        .executor
        .spawn_critical_async("main-rpc", spawn_rpc(deps));
    Ok(())
}

/// Spawns the RPC server.
async fn spawn_rpc(deps: RpcDeps) -> Result<()> {
    let mut module = RpcModule::new(());

    // Register existing protocol version method
    let _ = module.register_method("strata_protocolVersion", |_, _, _ctx| {
        Ok::<u32, ErrorObjectOwned>(1)
    });

    // Create and register OL RPC server
    let ol_rpc_server = OLRpcServer::new(deps.storage, deps.status_channel, deps.mempool_handle);
    let ol_module = OLClientRpcServer::into_rpc(ol_rpc_server);
    module
        .merge(ol_module)
        .map_err(|e| anyhow!("Failed to merge OL RPC module: {}", e))?;

    let addr = format!("{}:{}", deps.rpc_host, deps.rpc_port);
    let rpc_server = ServerBuilder::new()
        .build(&addr)
        .await
        .map_err(|e| anyhow!("Failed to build RPC server on {addr}: {e}"))?;

    let rpc_handle = rpc_server.start(module);

    // wait for rpc to stop
    rpc_handle.stopped().await;

    Ok(())
}
