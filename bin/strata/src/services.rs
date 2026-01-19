//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use strata_chain_worker_new::{ChainWorkerBuilder, ChainWorkerContextImpl};
use strata_common::ws_client::{ManagedWsClient, WsClientConfig};
use strata_consensus_logic::{
    message::ForkChoiceMessage,
    sync_manager::{spawn_asm_worker, spawn_csm_listener},
};
use strata_identifiers::OLBlockCommitment;
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};
use strata_ol_sync::{OLRpcSyncPeer, OLSyncContext};
use strata_rpc_api_new::OLClientRpcServer;
use tokio::sync::mpsc::{self, Sender};
use tracing::info;

use crate::{context::NodeContext, errors::InitError, rpc::OLRpcServer, run_context::RunContext};

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

    // Start Chain worker
    let chain_worker_context = ChainWorkerContextImpl::new(
        nodectx.storage.ol_block().clone(),
        nodectx.storage.ol_state().clone(),
        nodectx.storage.checkpoint().clone(),
    );
    let chain_worker_handle = ChainWorkerBuilder::new()
        .with_context(chain_worker_context)
        .with_params(nodectx.params.clone())
        .with_status_channel((*nodectx.status_channel).clone())
        .with_runtime(nodectx.executor.handle().clone())
        .launch(&nodectx.executor)?;
    let chain_worker_handle = Arc::new(chain_worker_handle);

    // FIXME: This is a placeholder fcm handler till we have properly migrated fcm for new chain
    // worker
    let (fcm_tx, _) = mpsc::channel(100);

    if nodectx.config.client.is_sequencer {
        start_sequencer_services(&nodectx)?;
    } else {
        start_fullnode_services(&nodectx, fcm_tx)?;
    }
    Ok(RunContext {
        runtime: nodectx.runtime,
        config: nodectx.config,
        params: nodectx.params,
        task_manager: nodectx.task_manager,
        executor: nodectx.executor,
        asm_handle,
        csm_monitor,
        mempool_handle,
        chain_worker_handle,
        storage: nodectx.storage,
        status_channel: nodectx.status_channel,
    })
}

fn start_fullnode_services(nodectx: &NodeContext, fcm_tx: Sender<ForkChoiceMessage>) -> Result<()> {
    let endpoint = nodectx.config.client.sync_endpoint.clone();
    let sync_endpoint = endpoint.ok_or(InitError::MissingSyncEndpoint)?;
    info!(?sync_endpoint, "initing fullnode task");

    let rpc_client = ManagedWsClient::new_with_default_pool(WsClientConfig { url: sync_endpoint });
    let sync_peer = OLRpcSyncPeer::new(rpc_client, 10);
    let ol_sync_context = OLSyncContext::new(
        sync_peer,
        nodectx.storage.clone(),
        nodectx.status_channel.clone(),
        fcm_tx,
    );

    // TODO: sync_worker as a service?
    nodectx
        .executor
        .spawn_critical_async("l2-sync-manager", async move {
            strata_ol_sync::sync_worker(&ol_sync_context)
                .await
                .map_err(Into::into)
        });
    Ok(())
}

fn start_sequencer_services(_nodectx: &NodeContext) -> Result<()> {
    // TODO: Start other tasks like l1writer, broadcaster
    Ok(())
}

/// Starts the mempool service.
fn start_mempool(nodectx: &NodeContext) -> Result<MempoolHandle> {
    let config = OLMempoolConfig::default();

    // Get current chain tip - try status channel first, fall back to genesis from storage
    let current_tip = match nodectx.status_channel.get_chain_sync_status() {
        Some(status) => status.tip,
        None => {
            // No chain sync status yet - get genesis block from OL storage
            let genesis_blocks = nodectx
                .storage
                .ol_block()
                .get_blocks_at_height_blocking(0)
                .map_err(|e| anyhow!("Failed to get genesis block: {e}"))?;
            let genesis_blkid = genesis_blocks
                .first()
                .ok_or_else(|| anyhow!("Genesis block not found, cannot start mempool"))?;
            OLBlockCommitment::new(0, *genesis_blkid)
        }
    };

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
