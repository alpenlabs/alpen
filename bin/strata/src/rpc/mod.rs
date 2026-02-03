//! OL RPC server implementation.

mod errors;
mod node_rpc;
mod seq_rpc;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use node_rpc::*;
use seq_rpc::*;
use strata_btcio::writer::EnvelopeHandle;
use strata_ol_mempool::MempoolHandle;
use strata_ol_rpc_api::{OLClientRpcServer, OLSequencerRpcServer};
use strata_ol_sequencer::{BlockasmHandle, TemplateManager};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::run_context::RunContext;

/// Dependencies needed by the RPC server.
/// Grouped to reduce parameter count when spawning the RPC task.
struct RpcDeps {
    rpc_host: String,
    rpc_port: u16,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    mempool_handle: Arc<MempoolHandle>,
    seq_deps: Option<SeqRpcDeps>,
}

/// Dependencies required for sequencer specific rpc endpoints
struct SeqRpcDeps {
    blockasm_handle: Arc<BlockasmHandle>,
    envelope_handle: Arc<EnvelopeHandle>,
}

impl SeqRpcDeps {
    fn new(blockasm_handle: Arc<BlockasmHandle>, envelope_handle: Arc<EnvelopeHandle>) -> Self {
        Self {
            blockasm_handle,
            envelope_handle,
        }
    }

    fn blockasm_handle(&self) -> &Arc<BlockasmHandle> {
        &self.blockasm_handle
    }

    fn envelope_handle(&self) -> &Arc<EnvelopeHandle> {
        &self.envelope_handle
    }
}

/// Starts the RPC server.
pub(crate) fn start_rpc(runctx: &RunContext) -> Result<()> {
    // Bundle RPC dependencies from context for the async task
    let deps = RpcDeps {
        rpc_host: runctx.config().client.rpc_host.clone(),
        rpc_port: runctx.config().client.rpc_port,
        storage: runctx.storage().clone(),
        status_channel: runctx.status_channel().clone(),
        mempool_handle: runctx.mempool_handle().clone(),
        seq_deps: runctx
            .sequencer_handles()
            .as_ref()
            .map(|s| SeqRpcDeps::new(s.blockasm_handle().clone(), s.envelope_handle().clone())),
    };

    runctx
        .executor()
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
    let ol_rpc_server = OLRpcServer::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle,
    );
    let ol_module = OLClientRpcServer::into_rpc(ol_rpc_server);
    module
        .merge(ol_module)
        .map_err(|e| anyhow!("Failed to merge OL RPC module: {}", e))?;

    // Create sequencer rpc handler if sequencer
    if let Some(seq_deps) = deps.seq_deps {
        let block_template_cache_ttl = Duration::from_millis(60000); // One minute
        let tmp_mgr = TemplateManager::new(
            seq_deps.blockasm_handle().clone(),
            deps.storage.clone(),
            block_template_cache_ttl,
        );
        let ol_seq_server = OLSeqRpcServer::new(
            deps.storage.clone(),
            deps.status_channel.clone(),
            tmp_mgr.into(),
            seq_deps.envelope_handle().clone(),
        );
        let ol_seq_module = OLSequencerRpcServer::into_rpc(ol_seq_server);
        module
            .merge(ol_seq_module)
            .map_err(|e| anyhow!("Failed to merge OL sequencer RPC module: {}", e))?;
    }

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
