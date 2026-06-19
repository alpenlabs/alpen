//! OL RPC server implementation.

mod auth;
pub(crate) mod errors;
mod node;
#[cfg(test)]
mod node_tests;
mod provider;

use std::{env, sync::Arc};

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use node::*;
use provider::NodeRpcProvider;
#[cfg(feature = "sequencer")]
use strata_btcio::writer::{EnvelopeHandle, EnvelopeSigningModeProvider};
#[cfg(feature = "debug-utils")]
use strata_common::{BAIL_SENDER, KNOWN_BAIL_TAGS};
use strata_config::SecretString;
#[cfg(feature = "sequencer")]
use strata_consensus_logic::FcmServiceHandle;
use strata_identifiers::L1Height;
#[cfg(feature = "sequencer")]
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_mempool::MempoolHandle;
#[cfg(feature = "sequencer")]
use strata_ol_rpc_api::OLSequencerRpcServer;
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer, OLSubmitRpcServer};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::info;

#[cfg(feature = "sequencer")]
use crate::checkpoint_auth::CheckpointSequencerKeyProvider;
use crate::run_context::{NodeRole, RunContext};
#[cfg(feature = "sequencer")]
use crate::sequencer::OLSeqRpcServer;

const STRATA_RPC_PERMISSIVE_CORS_ENV_VAR: &str = "STRATA_RPC_PERMISSIVE_CORS";

/// Dependencies needed by the RPC server.
/// Grouped to reduce parameter count when spawning the RPC task.
#[derive(Clone)]
struct RpcDeps {
    rpc_host: String,
    rpc_port: u16,
    admin_rpc_host: String,
    admin_rpc_port: u16,
    admin_rpc_bearer_token: Option<SecretString>,
    submit_rpc_host: String,
    submit_rpc_port: u16,
    submit_rpc_bearer_token: Option<SecretString>,
    genesis_l1_height: L1Height,
    max_headers_range: usize,
    node_role: NodeRole,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    /// [`None`] on checkpoint-sync nodes; `submit_transaction` returns an
    /// unavailable error in that case.
    mempool_handle: Option<Arc<MempoolHandle>>,
    #[cfg(feature = "sequencer")]
    seq_deps: Option<SeqRpcDeps>,
}

/// Dependencies required for sequencer specific rpc endpoints
#[cfg(feature = "sequencer")]
#[derive(Clone)]
struct SeqRpcDeps {
    /// Envelope handle.
    envelope_handle: Arc<EnvelopeHandle>,

    /// Block assembly handle.
    blockasm_handle: Arc<BlockasmHandle>,

    /// Fork-choice manager handle.
    fcm_handle: Arc<FcmServiceHandle>,

    /// Source for verifying reveal-tx signatures submitted via RPC.
    signing_mode_provider: Arc<dyn EnvelopeSigningModeProvider>,
}

#[cfg(feature = "sequencer")]
impl SeqRpcDeps {
    /// Creates a new [`SeqRpcDeps`] instance.
    fn new(
        envelope_handle: Arc<EnvelopeHandle>,
        blockasm_handle: Arc<BlockasmHandle>,
        fcm_handle: Arc<FcmServiceHandle>,
        signing_mode_provider: Arc<dyn EnvelopeSigningModeProvider>,
    ) -> Self {
        Self {
            envelope_handle,
            blockasm_handle,
            fcm_handle,
            signing_mode_provider,
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

    /// Returns the fork-choice manager handle.
    fn fcm_handle(&self) -> &Arc<FcmServiceHandle> {
        &self.fcm_handle
    }

    fn signing_mode_provider(&self) -> &Arc<dyn EnvelopeSigningModeProvider> {
        &self.signing_mode_provider
    }
}

fn rpc_permissive_cors_enabled() -> Result<bool> {
    match env::var(STRATA_RPC_PERMISSIVE_CORS_ENV_VAR) {
        Ok(value) => {
            if !value.is_ascii() {
                return Err(anyhow!(
                    "{STRATA_RPC_PERMISSIVE_CORS_ENV_VAR} must be ASCII"
                ));
            }

            match value.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(anyhow!(
                    "{STRATA_RPC_PERMISSIVE_CORS_ENV_VAR} must be one of \
                     1/true/yes/on or 0/false/no/off"
                )),
            }
        }
        Err(env::VarError::NotPresent) => Ok(false),
        Err(env::VarError::NotUnicode(_)) => Err(anyhow!(
            "{STRATA_RPC_PERMISSIVE_CORS_ENV_VAR} must be ASCII"
        )),
    }
}

/// Starts the RPC server.
pub(crate) fn start_rpc(runctx: &RunContext) -> Result<()> {
    // Bundle RPC dependencies from context for the async task
    #[cfg(feature = "sequencer")]
    let seq_deps = runctx.sequencer_handles().map(|handles| {
        // A sequencer node always runs FCM — start_sync_services produces
        // SyncServiceHandle::Fcm on the is_sequencer branch.
        let fcm_handle = runctx
            .fcm_handle()
            .expect("sequencer node must have an FCM sync handle")
            .clone();
        SeqRpcDeps::new(
            handles.envelope_handle().clone(),
            handles.blockasm_handle().clone(),
            fcm_handle,
            Arc::new(CheckpointSequencerKeyProvider::new(
                runctx.storage().clone(),
            )),
        )
    });

    let deps = RpcDeps {
        rpc_host: runctx.config().client.rpc_host.clone(),
        rpc_port: runctx.config().client.rpc_port,
        admin_rpc_host: runctx.config().client.admin_rpc_host.clone(),
        admin_rpc_port: runctx.config().client.admin_rpc_port,
        admin_rpc_bearer_token: runctx.config().client.admin_rpc_bearer_token.clone(),
        submit_rpc_host: runctx.config().client.submit_rpc_host.clone(),
        submit_rpc_port: runctx.config().client.submit_rpc_port,
        submit_rpc_bearer_token: runctx.config().client.submit_rpc_bearer_token.clone(),
        genesis_l1_height: runctx.asm_params().anchor.block.height(),
        max_headers_range: runctx.config().client.max_headers_range,
        node_role: runctx.node_role(),
        storage: runctx.storage().clone(),
        status_channel: runctx.status_channel().clone(),
        mempool_handle: runctx.mempool_handle().cloned(),
        #[cfg(feature = "sequencer")]
        seq_deps,
    };

    runctx
        .executor()
        .spawn_critical_async("main-rpc", spawn_public_rpc(deps.clone()));
    if deps.node_role.serves_sequencer_rpc() {
        runctx
            .executor()
            .spawn_critical_async("admin-rpc", spawn_admin_rpc(deps.clone()));
        runctx
            .executor()
            .spawn_critical_async("submit-rpc", spawn_submit_rpc(deps));
    }
    Ok(())
}

fn build_public_rpc_module(deps: &RpcDeps) -> Result<RpcModule<()>> {
    let mut module = build_public_static_rpc_module();

    register_client_rpc(&mut module, deps)?;
    if deps.node_role.serves_fullnode_rpc() {
        register_fullnode_rpc(&mut module, deps)?;
    }

    Ok(module)
}

fn register_client_rpc(module: &mut RpcModule<()>, deps: &RpcDeps) -> Result<()> {
    let client_provider = NodeRpcProvider::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle.clone(),
    );
    let ol_rpc_server = OLRpcServer::new(
        client_provider,
        deps.genesis_l1_height,
        deps.max_headers_range,
    );
    let ol_module = OLClientRpcServer::into_rpc(ol_rpc_server);
    module
        .merge(ol_module)
        .map_err(|e| anyhow!("Failed to merge OL RPC module: {}", e))
}

fn register_fullnode_rpc(module: &mut RpcModule<()>, deps: &RpcDeps) -> Result<()> {
    let fullnode_provider = NodeRpcProvider::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle.clone(),
    );
    let ol_fullnode_listener = OLRpcServer::new(
        fullnode_provider,
        deps.genesis_l1_height,
        deps.max_headers_range,
    );
    let ol_fullnode_module = OLFullNodeRpcServer::into_rpc(ol_fullnode_listener);
    module
        .merge(ol_fullnode_module)
        .map_err(|e| anyhow!("Failed to merge OL fullnode RPC module: {}", e))
}

fn build_public_static_rpc_module() -> RpcModule<()> {
    let mut module = RpcModule::new(());

    // Register existing protocol version method
    let _ = module.register_method("strata_protocolVersion", |_, _, _ctx| {
        Ok::<u32, ErrorObjectOwned>(1)
    });

    #[cfg(feature = "debug-utils")]
    {
        let _ = module.register_method("debug_bail", |params, _, _| {
            let ctx: String = params.one()?;
            let _ = BAIL_SENDER.send(Some(ctx));
            Ok::<(), ErrorObjectOwned>(())
        });

        // Returns the registered bail tag identifiers. Functional tests use
        // this to validate tag strings without maintaining a Python-side
        // mirror of the Rust constants in `strata_common::bail_tags`.
        let _ = module.register_method("debug_listBailTags", |_, _, _| {
            Ok::<Vec<&'static str>, ErrorObjectOwned>(KNOWN_BAIL_TAGS.to_vec())
        });
    }

    module
}

fn build_admin_rpc_module(deps: &RpcDeps) -> Result<RpcModule<()>> {
    let mut module = RpcModule::new(());

    // Create sequencer rpc handler if running as sequencer
    #[cfg(feature = "sequencer")]
    if let Some(sequencer_deps) = deps.seq_deps.as_ref() {
        let ol_seq_listener = OLSeqRpcServer::new(
            deps.storage.clone(),
            deps.status_channel.clone(),
            sequencer_deps.blockasm_handle().clone(),
            sequencer_deps.envelope_handle().clone(),
            sequencer_deps.fcm_handle().clone(),
            sequencer_deps.signing_mode_provider().clone(),
        );
        let ol_seq_module = OLSequencerRpcServer::into_rpc(ol_seq_listener);
        module
            .merge(ol_seq_module)
            .map_err(|e| anyhow!("Failed to merge OL sequencer RPC module: {}", e))?;
    }

    Ok(module)
}

fn build_submit_rpc_module(deps: &RpcDeps) -> Result<RpcModule<OLRpcServer<NodeRpcProvider>>> {
    let submit_provider = NodeRpcProvider::new(
        deps.storage.clone(),
        deps.status_channel.clone(),
        deps.mempool_handle.clone(),
    );
    let submit_listener = OLRpcServer::new(
        submit_provider,
        deps.genesis_l1_height,
        deps.max_headers_range,
    );

    Ok(<OLRpcServer<NodeRpcProvider> as OLSubmitRpcServer>::into_rpc(submit_listener))
}

/// Spawns the public RPC server.
async fn spawn_public_rpc(deps: RpcDeps) -> Result<()> {
    let module = build_public_rpc_module(&deps)?;
    let addr = format!("{}:{}", deps.rpc_host, deps.rpc_port);
    let cors = if rpc_permissive_cors_enabled()? {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
    };
    let http_middleware = ServiceBuilder::new().layer(cors);
    info!(%addr, "starting public RPC server");
    let rpc_server = ServerBuilder::new()
        .set_http_middleware(http_middleware)
        .build(&addr)
        .await
        .map_err(|e| anyhow!("Failed to build public RPC server on {addr}: {e}"))?;

    let rpc_handle = rpc_server.start(module);

    rpc_handle.stopped().await;

    Ok(())
}

/// Spawns the admin RPC server.
async fn spawn_admin_rpc(deps: RpcDeps) -> Result<()> {
    let module = build_admin_rpc_module(&deps)?;
    let addr = format!("{}:{}", deps.admin_rpc_host, deps.admin_rpc_port);
    info!(%addr, "starting admin RPC server");
    let token = deps
        .admin_rpc_bearer_token
        .clone()
        .ok_or_else(|| anyhow!("client.admin_rpc_bearer_token must be set"))?;
    let auth_layer = ServiceBuilder::new().layer(auth::BearerAuthLayer::new(token.expose_secret()));
    let rpc_server = ServerBuilder::new()
        .set_http_middleware(auth_layer)
        .build(&addr)
        .await
        .map_err(|e| anyhow!("Failed to build admin RPC server on {addr}: {e}"))?;

    let rpc_handle = rpc_server.start(module);
    rpc_handle.stopped().await;

    Ok(())
}

/// Spawns the submit RPC server.
async fn spawn_submit_rpc(deps: RpcDeps) -> Result<()> {
    let module = build_submit_rpc_module(&deps)?;
    let addr = format!("{}:{}", deps.submit_rpc_host, deps.submit_rpc_port);
    info!(%addr, "starting submit RPC server");
    let token = deps
        .submit_rpc_bearer_token
        .clone()
        .ok_or_else(|| anyhow!("client.submit_rpc_bearer_token must be set"))?;
    let auth_layer = ServiceBuilder::new().layer(auth::BearerAuthLayer::new(token.expose_secret()));
    let rpc_server = ServerBuilder::new()
        .set_http_middleware(auth_layer)
        .build(&addr)
        .await
        .map_err(|e| anyhow!("Failed to build submit RPC server on {addr}: {e}"))?;

    let rpc_handle = rpc_server.start(module);
    rpc_handle.stopped().await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_static_rpc_module_does_not_include_admin_methods() {
        let module = build_public_static_rpc_module();
        assert!(
            !module
                .method_names()
                .any(|method| method.contains("strataadmin_"))
        );
    }

    #[test]
    fn public_static_rpc_module_does_not_include_submit_transaction() {
        let module = build_public_static_rpc_module();
        assert!(
            !module
                .method_names()
                .any(|method| method == "strata_submitTransaction")
        );
    }
}
