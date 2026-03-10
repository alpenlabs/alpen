//! OL RPC server implementation.

pub(crate) mod errors;
mod node;

use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use node::*;
#[cfg(feature = "sequencer")]
use strata_btcio::writer::EnvelopeHandle;
#[cfg(feature = "sequencer")]
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_mempool::MempoolHandle;
#[cfg(feature = "sequencer")]
use strata_ol_rpc_api::OLSequencerRpcServer;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::run_context::RunContext;
#[cfg(feature = "sequencer")]
use crate::sequencer::OLSeqRpcServer;

/// Dependencies needed by the RPC server.
/// Grouped to reduce parameter count when spawning the RPC task.
struct RpcDeps {
    rpc_host: String,
    rpc_port: u16,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    mempool_handle: Arc<MempoolHandle>,
    #[cfg(feature = "sequencer")]
    seq_deps: Option<SeqRpcDeps>,
}

/// Dependencies required for sequencer specific rpc endpoints
#[cfg(feature = "sequencer")]
struct SeqRpcDeps {
    /// Envelope handle.
    envelope_handle: Arc<EnvelopeHandle>,

    /// Block assembly handle.
    blockasm_handle: Arc<BlockasmHandle>,
}

#[cfg(feature = "sequencer")]
impl SeqRpcDeps {
    /// Creates a new [`SeqRpcDeps`] instance.
    fn new(envelope_handle: Arc<EnvelopeHandle>, blockasm_handle: Arc<BlockasmHandle>) -> Self {
        Self {
            envelope_handle,
            blockasm_handle,
        }
    }

    /// Returns the envelope handle.
    fn envelope_handle(&self) -> &Arc<EnvelopeHandle> {
        &self.envelope_handle
    }

    /// Returns the block assembly handle.
    fn blockasm_handle(&self) -> &Arc<BlockasmHandle> {
        &self.blockasm_handle
    }
}

/// Starts the RPC server.
pub(crate) fn start_rpc(runctx: &RunContext) -> Result<()> {
    // Bundle RPC dependencies from context for the async task
    #[cfg(feature = "sequencer")]
    let seq_deps = runctx.sequencer_handles().map(|handles| {
        SeqRpcDeps::new(
            handles.envelope_handle().clone(),
            handles.blockasm_handle().clone(),
        )
    });

    let deps = RpcDeps {
        rpc_host: runctx.config().client.rpc_host.clone(),
        rpc_port: runctx.config().client.rpc_port,
        storage: runctx.storage().clone(),
        status_channel: runctx.status_channel().clone(),
        mempool_handle: runctx.mempool_handle().clone(),
        #[cfg(feature = "sequencer")]
        seq_deps,
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

    // Create and register OL client RPC server
    let ol_rpc_server = OLRpcServer::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle.clone(),
    );
    let ol_module = OLClientRpcServer::into_rpc(ol_rpc_server);
    module
        .merge(ol_module)
        .map_err(|e| anyhow!("Failed to merge OL RPC module: {}", e))?;

    // Create and register OL fullnode RPC listener
    let ol_fullnode_listener = OLRpcServer::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle.clone(),
    );
    let ol_fullnode_module = OLFullNodeRpcServer::into_rpc(ol_fullnode_listener);
    module
        .merge(ol_fullnode_module)
        .map_err(|e| anyhow!("Failed to merge OL fullnode RPC module: {}", e))?;

    // Create sequencer rpc handler if running as sequencer
    #[cfg(feature = "sequencer")]
    if let Some(sequencer_deps) = deps.seq_deps {
        let ol_seq_listener = OLSeqRpcServer::new(
            deps.storage.clone(),
            deps.status_channel.clone(),
            sequencer_deps.blockasm_handle().clone(),
            sequencer_deps.envelope_handle().clone(),
        );
        let ol_seq_module = OLSequencerRpcServer::into_rpc(ol_seq_listener);
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
