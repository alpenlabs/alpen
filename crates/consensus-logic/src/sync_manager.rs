//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_asm_worker::{AsmWorkerHandle, AsmWorkerStatus};
use strata_chain_worker::ChainWorkerHandle;
use strata_csm_worker::{CsmWorkerService, CsmWorkerState};
use strata_eectl::{engine::ExecEngineCtl, handle::ExecCtlHandle};
use strata_primitives::params::Params;
use strata_service::{ServiceBuilder, SyncAsyncInput};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::{runtime::Handle, sync::mpsc};

use crate::{
    asm_worker_context::AsmWorkerCtx,
    chain_worker_context::ChainWorkerCtx,
    exec_worker_context::ExecWorkerCtx,
    fork_choice_manager::{self},
    message::ForkChoiceMessage,
};

/// Handle to the core pipeline tasks.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impls"
)]
pub struct SyncManager {
    params: Arc<Params>,
    fc_manager_tx: mpsc::Sender<ForkChoiceMessage>,
    asm_controller: Arc<AsmWorkerHandle>,
    status_channel: StatusChannel,
}

impl SyncManager {
    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn get_params(&self) -> Arc<Params> {
        self.params.clone()
    }

    pub fn get_asm_ctl(&self) -> Arc<AsmWorkerHandle> {
        self.asm_controller.clone()
    }

    pub fn status_channel(&self) -> &StatusChannel {
        &self.status_channel
    }

    /// Submits a fork choice message if possible. (synchronously)
    pub fn submit_chain_tip_msg(&self, ctm: ForkChoiceMessage) -> bool {
        self.fc_manager_tx.blocking_send(ctm).is_ok()
    }

    /// Submits a fork choice message if possible. (asynchronously)
    pub async fn submit_chain_tip_msg_async(&self, ctm: ForkChoiceMessage) -> bool {
        self.fc_manager_tx.send(ctm).await.is_ok()
    }
}

/// Starts the sync tasks using provided settings.
pub fn start_sync_tasks<E: ExecEngineCtl + Sync + Send + 'static>(
    executor: &TaskExecutor,
    storage: &Arc<NodeStorage>,
    bitcoin_client: Arc<Client>,
    engine: Arc<E>,
    params: Arc<Params>,
    status_channel: StatusChannel,
) -> anyhow::Result<SyncManager> {
    // Create channels.
    let (fcm_tx, fcm_rx) = mpsc::channel::<ForkChoiceMessage>(64);

    // Exec worker.
    let ex_storage = storage.clone();
    let ex_st_ch = status_channel.clone();
    let ex_handle = executor.handle().clone();
    let ex_handle = spawn_exec_worker(executor, ex_handle, ex_storage, ex_st_ch, engine)?;

    // Chain worker.
    let cw_handle = executor.handle().clone();
    let cw_storage = storage.clone();
    let cw_st_ch = status_channel.clone();
    let cw_params = params.clone();
    let cw_handle = Arc::new(spawn_chain_worker(
        executor, cw_handle, cw_storage, cw_st_ch, cw_params, ex_handle,
    )?);

    // ASM worker.
    let asm_handle = executor.handle().clone();
    let asm_storage = storage.clone();
    let asm_params = params.clone();
    let asm_controller = Arc::new(spawn_asm_worker(
        executor,
        asm_handle,
        asm_storage,
        asm_params,
        bitcoin_client,
    )?);

    // Launch CSM listener service that listens to ASM status updates
    let csm_params = params.clone();
    let csm_storage = storage.clone();
    let csm_st_ch = status_channel.clone();
    let csm_asm_monitor = asm_controller.get_monitor();

    spawn_csm_listener(
        executor,
        csm_params,
        csm_storage,
        csm_st_ch,
        csm_asm_monitor,
    )?;

    // Start the fork choice manager thread.  If we haven't done genesis yet
    // this will just wait until the CSM says we have.
    let fcm_storage = storage.clone();
    let fcm_params = params.clone();
    let fcm_handle = executor.handle().clone();
    let st_ch = status_channel.clone();
    executor.spawn_critical("fork_choice_manager::tracker_task", move |shutdown| {
        // TODO this should be simplified into a builder or something
        fork_choice_manager::tracker_task(
            shutdown,
            fcm_handle,
            fcm_storage,
            fcm_rx,
            cw_handle,
            fcm_params,
            st_ch,
        )
    });

    Ok(SyncManager {
        params,
        fc_manager_tx: fcm_tx,
        asm_controller,
        status_channel,
    })
}

fn spawn_csm_listener(
    executor: &TaskExecutor,
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    asm_monitor: &strata_service::ServiceMonitor<AsmWorkerStatus>,
) -> anyhow::Result<()> {
    // Create CSM worker state
    let csm_state = CsmWorkerState::new(params, storage, status_channel)?;

    // Create input that listens to ASM status updates via MPSC
    // This spawns a forwarder task that converts watch channel updates to MPSC messages
    let async_input = asm_monitor.create_listener_input(executor);

    // Wrap in SyncAsyncInput adapter since CSM worker is a sync service
    let csm_input = SyncAsyncInput::new(async_input, executor.handle().clone());

    // Launch the CSM worker service (which acts as a listener to ASM worker)
    let _csm_monitor = ServiceBuilder::<CsmWorkerService, _>::new()
        .with_state(csm_state)
        .with_input(csm_input)
        .launch_sync("csm_worker", executor)?;

    Ok(())
}

fn spawn_exec_worker<E: ExecEngineCtl + Sync + Send + 'static>(
    executor: &TaskExecutor,
    handle: Handle,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    engine: Arc<E>,
) -> anyhow::Result<ExecCtlHandle> {
    // Create the worker context - this stays in consensus-logic since it implements WorkerContext
    let context = ExecWorkerCtx::new(storage.l2().clone(), storage.client_state().clone());

    let handle = strata_eectl::builder::ExecWorkerBuilder::new()
        .with_context(context)
        .with_engine(engine)
        .with_status_channel(status_channel)
        .with_runtime(handle)
        .launch(executor)?;

    Ok(handle)
}

fn spawn_chain_worker(
    executor: &TaskExecutor,
    handle: Handle,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    params: Arc<Params>,
    exec_ctl_handle: ExecCtlHandle,
) -> anyhow::Result<ChainWorkerHandle> {
    // Create the worker context - this stays in consensus-logic since it implements WorkerContext
    let context = ChainWorkerCtx::new(
        storage.l2().clone(),
        storage.chainstate().clone(),
        storage.checkpoint().clone(),
        0, // FIXME: Not sure what this is
    );

    // Use the new builder API to launch the worker and get a handle
    let handle = strata_chain_worker::ChainWorkerBuilder::new()
        .with_context(context)
        .with_params(params)
        .with_exec_handle(exec_ctl_handle)
        .with_status_channel(status_channel)
        .with_runtime(handle)
        .launch(executor)?;

    Ok(handle)
}

fn spawn_asm_worker(
    executor: &TaskExecutor,
    handle: Handle,
    storage: Arc<NodeStorage>,
    params: Arc<Params>,
    bitcoin_client: Arc<Client>,
) -> anyhow::Result<AsmWorkerHandle> {
    // This feels weird to pass both L1BlockManager and Bitcoin client, but ASM consumes raw bitcoin
    // blocks while following canonical chain (and "canonicity" of l1 chain is imposed by the l1
    // block manager).
    let context = AsmWorkerCtx::new(
        handle.clone(),
        bitcoin_client,
        storage.l1().clone(),
        storage.asm().clone(),
    );

    // Use the new builder API to launch the worker and get a handle.
    let handle = strata_asm_worker::AsmWorkerBuilder::new()
        .with_context(context)
        .with_params(params)
        .with_runtime(handle)
        .launch(executor)?;

    Ok(handle)
}
