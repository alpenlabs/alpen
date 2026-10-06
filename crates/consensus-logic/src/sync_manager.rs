//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_asm_params::AsmParams;
use strata_asm_spec::host::{build_execution_registry, CompiledSpec};
use strata_asm_worker::{AsmWorkerBuilder, AsmWorkerHandle, AsmWorkerStatus};
use strata_config::AsmExecutionParams;
use strata_csm_worker::{CsmWorkerService, CsmWorkerState, CsmWorkerStatus};
use strata_node_context::NodeContext;
use strata_primitives::prelude::L1BlockCommitment;
use strata_service::{ServiceBuilder, ServiceMonitor, SyncAsyncInput};
use strata_status::StatusChannel;
use strata_storage::{MmrId, NodeStorage};
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;

use crate::{asm_worker_context::AsmWorkerCtx, csm_worker_context::CsmWorkerContextImpl};

pub fn spawn_csm_listener_with_ctx(
    nodectx: &NodeContext,
    asm_monitor: &ServiceMonitor<AsmWorkerStatus>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    spawn_csm_listener(
        nodectx.executor(),
        nodectx.asm_params().clone(),
        nodectx.config().btcio.l1_reorg_safe_depth,
        nodectx.storage().clone(),
        nodectx.status_channel().clone(),
        asm_monitor,
        nodectx.bitcoin_client().clone(),
    )
}

fn spawn_csm_listener(
    executor: &TaskExecutor,
    asm_params: Arc<AsmParams>,
    l1_reorg_safe_depth: u32,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    asm_monitor: &ServiceMonitor<AsmWorkerStatus>,
    bitcoin_client: Arc<Client>,
) -> anyhow::Result<ServiceMonitor<CsmWorkerStatus>> {
    // Create CSM worker state.
    let ctx = CsmWorkerContextImpl::new(
        executor.handle().clone(),
        bitcoin_client,
        asm_params,
        l1_reorg_safe_depth,
        storage.clone(),
        status_channel,
    );
    let csm_state = CsmWorkerState::init_from_context(ctx)?;

    // Get the starting block from CSM's last processed block
    // If CSM hasn't processed any blocks yet, we get the latest ASM state from storage
    let from_block = if let Some(last_block) = csm_state.get_last_asm_block() {
        last_block
    } else {
        // Get the latest ASM state as fallback. This reads the anchor state
        // alone: on a fresh node the only entry is the genesis anchor, which
        // has no logs, so the combined read would come back empty.
        let (latest_block, _) = storage
            .asm()
            .fetch_most_recent_anchor_state_blocking()?
            .expect("No ASM state available");
        latest_block
    };

    // Fetch historical ASM states starting from the next height.
    let max_historical_blocks = 1000;
    let nh = from_block.height() + 1;
    let historical_states = storage.asm().get_states_from_blocking(
        L1BlockCommitment::new(nh, Default::default()),
        max_historical_blocks,
    )?;

    // Convert historical states to ASM worker status updates
    let initial_updates: Vec<AsmWorkerStatus> = historical_states
        .into_iter()
        .map(|(block, state)| AsmWorkerStatus {
            is_initialized: true,
            cur_block: Some(block),
            cur_state: Some(state.state().clone()),
        })
        .collect();

    // Create an input that listens to ASM status updates with historical prepended
    let async_input = asm_monitor.create_listener_input_with(executor, initial_updates);
    // Wrap in SyncAsyncInput adapter since CSM worker is a sync service.
    let csm_input = SyncAsyncInput::new(async_input, executor.handle().clone());

    // Launch the CSM worker service (which acts as a listener to ASM worker).
    let csm_monitor = ServiceBuilder::<CsmWorkerService<CsmWorkerContextImpl>, _>::new()
        .with_state(csm_state)
        .with_input(csm_input)
        .launch_sync("csm_worker", executor)?;

    Ok(csm_monitor)
}

pub fn spawn_asm_worker_with_ctx(nodectx: &NodeContext) -> anyhow::Result<AsmWorkerHandle> {
    spawn_asm_worker(
        nodectx.executor(),
        nodectx.executor().handle().clone(),
        nodectx.storage().clone(),
        nodectx.asm_params().clone(),
        nodectx.asm_execution(),
        nodectx.bitcoin_client().clone(),
    )
}

pub fn spawn_asm_worker(
    executor: &TaskExecutor,
    handle: Handle,
    storage: Arc<NodeStorage>,
    asm_params: Arc<AsmParams>,
    execution: &AsmExecutionParams,
    bitcoin_client: Arc<Client>,
) -> anyhow::Result<AsmWorkerHandle> {
    // This feels weird to pass both L1BlockManager and Bitcoin client, but ASM consumes raw bitcoin
    // blocks while following canonical chain (and "canonicity" of l1 chain is imposed by the l1
    // block manager).
    let mmr_handle = storage.mmr_index().get_handle(MmrId::Asm);

    let registry = build_execution_registry(
        execution
            .targets()
            .iter()
            .map(|target| (target.predicate().clone(), target.spec_id())),
    )?;
    let genesis_spec =
        CompiledSpec::resolve(registry.resolve(execution.genesis_predicate())?.spec_id())?;
    let genesis = genesis_spec.construct_genesis_state(&asm_params);

    let context = AsmWorkerCtx::new(
        handle.clone(),
        bitcoin_client,
        storage.l1().clone(),
        storage.asm().clone(),
        mmr_handle,
    );

    // The worker validates the anchor and prefills its manifest MMR.
    let handle = AsmWorkerBuilder::new()
        .with_context(context)
        .with_genesis(genesis, execution.genesis_predicate().clone())
        .with_registry(registry)
        .launch(executor)?;

    Ok(handle)
}
