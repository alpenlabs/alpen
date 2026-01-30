//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonrpsee::{RpcModule, server::ServerBuilder, types::ErrorObjectOwned};
use strata_btcio::{
    broadcaster::{L1BroadcastHandle, spawn_broadcaster_task},
    reader::query::bitcoin_data_reader_task,
    writer::{EnvelopeHandle, start_envelope_task},
};
use strata_chain_worker_new::{ChainWorkerBuilder, ChainWorkerContextImpl};
use strata_config::EpochSealingConfig;
use strata_consensus_logic::sync_manager::{spawn_asm_worker, spawn_csm_listener};
use strata_db_types::traits::DatabaseBackend;
use strata_identifiers::OLBlockCommitment;
use strata_ol_block_assembly::{
    BlockasmBuilder, BlockasmHandle, FixedSlotSealing, MempoolProviderImpl,
};
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};
use strata_rpc_api_new::OLClientRpcServer;
use strata_storage::ops::l1tx_broadcast;

use crate::{
    context::NodeContext, helpers::generate_sequencer_address, rpc::OLRpcServer,
    run_context::RunContext,
};

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
    // Start Asm worker and wrap in Arc for sharing with other services
    let asm_handle = Arc::new(spawn_asm_worker(
        &nodectx.executor,
        nodectx.executor.handle().clone(),
        nodectx.storage.clone(),
        Arc::new(nodectx.params.rollup.clone()),
        nodectx.bitcoin_client.clone(),
    )?);

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

    // Start btcio reader task (polls Bitcoin for new blocks and submits to ASM)
    start_btcio_reader(&nodectx, asm_handle.clone());

    // Sequencer-specific tasks
    let (broadcast_handle, envelope_handle, blockasm_handle) = if nodectx.config.client.is_sequencer
    {
        let broadcast_handle = Arc::new(start_broadcaster(&nodectx));
        let envelope_handle = start_writer(&nodectx, broadcast_handle.clone())?;
        let blockasm_handle = start_block_assembly(&nodectx, mempool_handle.clone())?;
        (
            Some(broadcast_handle),
            Some(envelope_handle),
            Some(blockasm_handle),
        )
    } else {
        (None, None, None)
    };

    // TODO: Start other tasks like fcm, etc. all as service, returning the monitors.

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
        broadcast_handle,
        envelope_handle,
        blockasm_handle,
        storage: nodectx.storage,
        status_channel: nodectx.status_channel,
    })
}

/// Starts the btcio reader task.
///
/// Polls Bitcoin for new blocks and submits them to ASM for processing.
fn start_btcio_reader(nodectx: &NodeContext, asm_handle: Arc<strata_asm_worker::AsmWorkerHandle>) {
    nodectx.executor.spawn_critical_async(
        "bitcoin_data_reader_task",
        bitcoin_data_reader_task(
            nodectx.bitcoin_client.clone(),
            nodectx.storage.clone(),
            Arc::new(nodectx.config.btcio.reader.clone()),
            nodectx.params.clone(),
            (*nodectx.status_channel).clone(),
            asm_handle,
        ),
    );
}

/// Starts the L1 broadcaster task (sequencer-specific).
///
/// Manages L1 transaction broadcasting and tracks confirmation status.
fn start_broadcaster(nodectx: &NodeContext) -> L1BroadcastHandle {
    let broadcast_db = nodectx.db.broadcast_db();
    let broadcast_ctx = l1tx_broadcast::Context::new(broadcast_db);
    let broadcast_ops = Arc::new(broadcast_ctx.into_ops(nodectx.pool.clone()));

    spawn_broadcaster_task(
        &nodectx.executor,
        nodectx.bitcoin_client.clone(),
        broadcast_ops,
        nodectx.params.clone(),
        nodectx.config.btcio.broadcaster.poll_interval_ms,
    )
}

/// Starts the L1 writer/envelope task (sequencer-specific).
///
/// Bundles L1 intents, creates envelope transactions, and publishes to Bitcoin.
fn start_writer(
    nodectx: &NodeContext,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> Result<Arc<EnvelopeHandle>> {
    let sequencer_address = nodectx
        .runtime
        .block_on(generate_sequencer_address(&nodectx.bitcoin_client))?;

    let writer_db = nodectx.db.writer_db();

    start_envelope_task(
        &nodectx.executor,
        nodectx.bitcoin_client.clone(),
        Arc::new(nodectx.config.btcio.writer.clone()),
        nodectx.params.clone(),
        sequencer_address,
        writer_db,
        (*nodectx.status_channel).clone(),
        nodectx.pool.clone(),
        broadcast_handle,
    )
}

/// Starts the OL block assembly service (sequencer-specific).
///
/// Assembles OL blocks from mempool transactions.
fn start_block_assembly(
    nodectx: &NodeContext,
    mempool_handle: MempoolHandle,
) -> Result<BlockasmHandle> {
    let sequencer_config = nodectx
        .config
        .sequencer
        .clone()
        .ok_or_else(|| anyhow!("Sequencer config required for block assembly"))?;

    let epoch_sealing_config = nodectx.config.epoch_sealing.clone().unwrap_or_default();

    let slots_per_epoch = match epoch_sealing_config {
        EpochSealingConfig::FixedSlot { slots_per_epoch } => slots_per_epoch,
    };

    let mempool_provider = MempoolProviderImpl::new(Arc::new(mempool_handle));
    let epoch_sealing = FixedSlotSealing::new(slots_per_epoch);
    let state_provider = nodectx.storage.ol_state().clone();

    nodectx.runtime.block_on(async {
        BlockasmBuilder::new(
            nodectx.params.clone(),
            nodectx.storage.clone(),
            mempool_provider,
            epoch_sealing,
            state_provider,
            sequencer_config,
        )
        .launch(&nodectx.executor)
        .await
    })
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
