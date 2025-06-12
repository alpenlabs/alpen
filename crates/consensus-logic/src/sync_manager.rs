//! High level sync manager which controls core sync tasks and manages sync
//! status.  Exposes handles to interact with fork choice manager and CSM
//! executor and other core sync pipeline tasks.

use std::sync::Arc;

use strata_eectl::engine::ExecEngineCtl;
use strata_primitives::params::Params;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::sync::{broadcast, mpsc};

use crate::{
    csm::{
        ctl::CsmController,
        message::{ClientUpdateNotif, CsmMessage, ForkChoiceMessage},
        worker,
    },
    fork_choice_manager,
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
    let csm_controller = Arc::new(CsmController::new(storage.sync_event().clone(), csm_tx));

    // TODO should this be in an `Arc`?  it's already fairly compact so we might
    // not be benefitting from the reduced cloning
    let (cupdate_tx, cupdate_rx) = broadcast::channel::<Arc<ClientUpdateNotif>>(64);

    // Start the fork choice manager thread.  If we haven't done genesis yet
    // this will just wait until the CSM says we have.
    let fcm_storage = storage.clone();
    let fcm_engine = engine.clone();
    let fcm_csm_controller = csm_controller.clone();
    let fcm_params = params.clone();
    let handle = executor.handle().clone();
    let st_ch = status_channel.clone();
    executor.spawn_critical("fork_choice_manager::tracker_task", move |shutdown| {
        // TODO this should be simplified into a builder or something
        fork_choice_manager::tracker_task(
            shutdown,
            handle,
            fcm_storage,
            fcm_engine,
            fcm_rx,
            fcm_csm_controller,
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

    Ok(SyncManager {
        params,
        fc_manager_tx: fcm_tx,
        csm_controller,
        cupdate_rx,
        status_channel,
    })
}
