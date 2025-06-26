//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::sync::Arc;

use strata_chain_worker::{ChainWorkerHandle, ChainWorkerInput, ChainWorkerMessage, WorkerShared};
use strata_chainexec::ChainExecutor;
use strata_eectl::{
    engine::ExecEngineCtl,
    handle::{ExecCtlHandle, ExecCtlInput},
    worker::{exec_worker_task, ExecWorkerState},
};
use strata_primitives::{l2::L2BlockCommitment, params::Params};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{ShutdownGuard, TaskExecutor};
use tokio::{
    runtime::Handle,
    sync::{broadcast, mpsc, Mutex},
};
use tracing::info;

use crate::{
    chain_worker_context::ChainWorkerCtx,
    csm::{
        ctl::CsmController,
        message::{ClientUpdateNotif, CsmMessage, ForkChoiceMessage},
        worker::{self},
    },
    exec_worker_context::ExecWorkerCtx,
    fork_choice_manager::{self},
};

/// Handle to the core pipeline tasks.
#[expect(missing_debug_implementations)]
pub struct SyncManager {
    params: Arc<Params>,
    fc_manager_tx: mpsc::Sender<ForkChoiceMessage>,
    csm_controller: Arc<CsmController>,
    cupdate_rx: broadcast::Receiver<Arc<ClientUpdateNotif>>,
    status_channel: StatusChannel,
}

impl SyncManager {
    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn get_params(&self) -> Arc<Params> {
        self.params.clone()
    }

    /// Gets a ref to the CSM controller.
    pub fn csm_controller(&self) -> &CsmController {
        &self.csm_controller
    }

    /// Gets a clone of the CSM controller.
    pub fn get_csm_ctl(&self) -> Arc<CsmController> {
        self.csm_controller.clone()
    }

    /// Returns a new broadcast `Receiver` handle to the consensus update
    /// notification queue.  Provides no guarantees about which position in the
    /// queue will be returned on the first receive.
    pub fn create_cstate_subscription(&self) -> broadcast::Receiver<Arc<ClientUpdateNotif>> {
        self.cupdate_rx.resubscribe()
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
#[allow(clippy::too_many_arguments)]
pub fn start_sync_tasks<E: ExecEngineCtl + Sync + Send + 'static>(
    executor: &TaskExecutor,
    storage: &Arc<NodeStorage>,
    engine: Arc<E>,
    params: Arc<Params>,
    status_channel: StatusChannel,
) -> anyhow::Result<SyncManager> {
    // Create channels.
    let (fcm_tx, fcm_rx) = mpsc::channel::<ForkChoiceMessage>(64);
    let (csm_tx, csm_rx) = mpsc::channel::<CsmMessage>(64);
    let (chain_msg_tx, chain_msg_rx) = mpsc::channel::<ChainWorkerMessage>(64);
    let (exec_tx, exec_rx) = strata_eectl::handle::make_handle_pair();
    let csm_controller = Arc::new(CsmController::new(storage.sync_event().clone(), csm_tx));

    // TODO should this be in an `Arc`?  it's already fairly compact so we might
    // not be benefitting from the reduced cloning
    let (cupdate_tx, cupdate_rx) = broadcast::channel::<Arc<ClientUpdateNotif>>(64);

    // Start the fork choice manager thread.  If we haven't done genesis yet
    // this will just wait until the CSM says we have.
    let fcm_storage = storage.clone();
    let _fcm_csm_controller = csm_controller.clone();
    let fcm_params = params.clone();
    let fcm_handle = executor.handle().clone();
    let st_ch = status_channel.clone();
    let cw_handle: Arc<ChainWorkerHandle> = Arc::new(ChainWorkerHandle::new(
        Arc::new(Mutex::new(WorkerShared::default())),
        chain_msg_tx,
    ));
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

    // Prepare the client worker state and start the thread for that.
    let client_worker_state = worker::WorkerState::open(
        params.clone(),
        storage.clone(),
        cupdate_tx,
        storage.checkpoint().clone(),
    )?;
    let csm_engine = engine.clone();
    let st_ch = status_channel.clone();

    executor.spawn_critical("client_worker_task", move |shutdown| {
        worker::client_worker_task(shutdown, client_worker_state, csm_engine, csm_rx, st_ch)
    });

    let cw_handle = executor.handle().clone();
    let cw_storage = storage.clone();
    let cw_status = status_channel.clone();
    let cw_params = params.clone();
    executor.spawn_critical("chain_worker_task", move |shutdown| {
        spawn_chain_worker(
            shutdown,
            cw_handle,
            cw_storage,
            cw_status,
            cw_params,
            exec_tx,
            chain_msg_rx,
        )
    });

    let ew_handle = executor.handle().clone();
    let ew_st_ch = status_channel.clone();
    let ew_storage = storage.clone();
    executor.spawn_critical("exec_worker_task", move |shutdown| {
        spawn_exec_worker(shutdown, ew_handle, ew_storage, ew_st_ch, engine, exec_rx)
    });

    Ok(SyncManager {
        params,
        fc_manager_tx: fcm_tx,
        csm_controller,
        cupdate_rx,
        status_channel,
    })
}

fn spawn_exec_worker<E: ExecEngineCtl + Sync + Send + 'static>(
    shutdown: ShutdownGuard,
    handle: Handle,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    engine: Arc<E>,
    exec_rx: ExecCtlInput,
) -> anyhow::Result<()> {
    info!("waiting until genesis");
    let init_state = handle.block_on(status_channel.wait_until_genesis())?;
    let cur_tip = match init_state.get_declared_final_epoch().cloned() {
        Some(epoch) => epoch.to_block_commitment(),
        None => L2BlockCommitment::new(
            0,
            *init_state.sync().expect("after genesis").genesis_blkid(),
        ),
    };

    let blkid = *cur_tip.blkid();
    info!(%blkid, "starting exec worker");

    let exec_env_id = ();
    let state = ExecWorkerState::new(engine, exec_env_id, cur_tip, cur_tip);
    let ctx = ExecWorkerCtx::new(storage.l2().clone());
    exec_worker_task(shutdown, state, exec_rx, &ctx)?;
    Ok(())
}

fn spawn_chain_worker(
    shutdown: ShutdownGuard,
    handle: Handle,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
    params: Arc<Params>,
    exect_ctl_handle: ExecCtlHandle,
    chain_msg_rx: mpsc::Receiver<ChainWorkerMessage>,
) -> anyhow::Result<()> {
    info!("waiting until genesis");
    let init_state = handle.block_on(status_channel.wait_until_genesis())?;
    let cur_tip = match init_state.get_declared_final_epoch().cloned() {
        Some(epoch) => epoch.to_block_commitment(),
        None => L2BlockCommitment::new(
            0,
            *init_state.sync().expect("after genesis").genesis_blkid(),
        ),
    };

    let blkid = *cur_tip.blkid();
    info!(%blkid, "starting chain worker");

    let context = ChainWorkerCtx::new(
        storage.l2().clone(),
        storage.chainstate().clone(),
        storage.checkpoint().clone(),
        0, // FIXME: Not sure what this is
    );
    let chain_exec = ChainExecutor::new(params.rollup().clone());
    let shared = Arc::new(Mutex::new(WorkerShared::default()));
    let state = strata_chain_worker::init_worker_state(
        shared.clone(),
        context,
        chain_exec,
        exect_ctl_handle,
        cur_tip,
    )?;
    let input = ChainWorkerInput::new(shared.clone(), chain_msg_rx);

    strata_chain_worker::worker_task(&shutdown, state, input)?;
    Ok(())
}
