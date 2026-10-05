//! Detects checkpoints this sequencer submitted that were mined on L1 but not accepted by the
//! ASM.
//!
//! The ASM emits nothing when it rejects a checkpoint, so a rejection only shows up as a mined
//! transaction whose epoch the CSM never sees accepted. The tracker follows the CSM's published
//! checkpoint state, walks the L1 writer's checkpoint bundles, resolves each reveal through the
//! broadcaster, and records a checkpoint as confirmed-but-rejected once its inclusion block is
//! buried at the reorg-safe depth while the ASM's verified epoch is still below the
//! checkpoint's epoch. Detection only: it changes no writer, broadcaster, or prover state.

mod classify;
mod context;
mod state;

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use strata_btcio::broadcaster::L1BroadcastHandle;
use strata_csm_types::CheckpointState;
use strata_db_types::l1_writer::BundleIdx;
use strata_service::{
    Response, Service, ServiceBuilder, ServiceMonitor, SyncAsyncInput, SyncService, TickMsg,
    TickingInput, TokioWatchInput,
};
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tracing::warn;

use self::context::{SubmissionTrackerContext, SubmissionTrackerContextImpl};
use self::state::SubmissionTrackerState;

/// How often the tracker re-checks without a new CSM update.
///
/// The broadcaster records confirmations on its own schedule, so the confirmation that makes a
/// rejection reportable can arrive after the CSM update that buried it.
const RECHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Status of the checkpoint submission tracker.
#[derive(Clone, Debug, Serialize)]
pub struct SubmissionTrackerStatus {
    /// First L1 writer bundle index not yet settled.
    pub scan_cursor: BundleIdx,

    /// Recorded rejections whose epoch the ASM has still not accepted.
    pub unresolved_rejections: usize,
}

/// Launches the checkpoint submission tracker.
///
/// Runs on sequencer nodes, which are the only ones that submit checkpoints. `l1_reorg_safe_depth`
/// is the node's checkpoint finality depth; a rejection is reported once the checkpoint's
/// inclusion block is that deep under the CSM tip.
pub fn launch_submission_tracker(
    storage: Arc<NodeStorage>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    checkpoint_state_rx: watch::Receiver<CheckpointState>,
    l1_reorg_safe_depth: u32,
    executor: &TaskExecutor,
) -> anyhow::Result<ServiceMonitor<SubmissionTrackerStatus>> {
    let runtime = executor.handle().clone();
    let ctx = SubmissionTrackerContextImpl::new(storage, broadcast_handle, runtime.clone());
    let latest = checkpoint_state_rx.borrow().clone();
    let state = SubmissionTrackerState::new(ctx, l1_reorg_safe_depth, latest)?;

    ServiceBuilder::<SubmissionTrackerService<SubmissionTrackerContextImpl>, _>::new()
        .with_state(state)
        .with_input(tracker_input(checkpoint_state_rx, runtime))
        .launch_sync("checkpoint_submission_tracker", executor)
        .map_err(|e| anyhow::anyhow!("failed to launch checkpoint submission tracker: {e}"))
}

/// Merges CSM checkpoint state updates with periodic rechecks.
///
/// The first tick fires immediately, which checks the state the tracker was seeded with.
fn tracker_input(
    checkpoint_state_rx: watch::Receiver<CheckpointState>,
    runtime: Handle,
) -> SyncAsyncInput<TickingInput<TokioWatchInput<CheckpointState>>> {
    // The interval registers with the runtime's timer when it is created, and callers may be
    // outside the runtime.
    let input = {
        let _runtime_guard = runtime.enter();
        TickingInput::new(
            RECHECK_INTERVAL,
            TokioWatchInput::from_receiver(checkpoint_state_rx),
        )
    };
    SyncAsyncInput::new(input, runtime)
}

/// Service wiring for [`SubmissionTrackerState`].
#[derive(Debug)]
struct SubmissionTrackerService<C>(PhantomData<C>);

impl<C: SubmissionTrackerContext> Service for SubmissionTrackerService<C> {
    type State = SubmissionTrackerState<C>;
    type Msg = TickMsg<CheckpointState>;
    type Status = SubmissionTrackerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        SubmissionTrackerStatus {
            scan_cursor: state.cursor(),
            unresolved_rejections: state.unresolved_rejections(),
        }
    }
}

impl<C: SubmissionTrackerContext> SyncService for SubmissionTrackerService<C> {
    fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        if let TickMsg::Msg(checkpoint_state) = input {
            state.set_latest(checkpoint_state);
        }
        // A failed check must not stop the node; the next update or tick retries it.
        if let Err(err) = state.evaluate() {
            warn!(?err, "checkpoint submission check failed");
        }
        Ok(Response::Continue)
    }
}

#[cfg(test)]
mod tests {
    use strata_csm_types::ClientState;
    use strata_identifiers::L1BlockCommitment;
    use tokio::runtime::Runtime;

    use super::*;

    #[test]
    fn input_builds_outside_the_runtime() {
        let runtime = Runtime::new().unwrap();
        let (_tx, rx) = watch::channel(CheckpointState::new(
            ClientState::default(),
            L1BlockCommitment::default(),
        ));
        tracker_input(rx, runtime.handle().clone());
    }
}
