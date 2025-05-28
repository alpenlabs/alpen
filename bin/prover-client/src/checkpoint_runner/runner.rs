use std::sync::Arc;

use strata_rocksdb::prover::db::ProofDb;
use tokio::{
    sync::Mutex,
    time::{interval, Duration},
};
use tracing::{error, info, warn};

use crate::{
    checkpoint_runner::fetch::fetch_latest_checkpoint_index,
    operators::{checkpoint::CheckpointOperator, ProvingOp},
    task_tracker::TaskTracker,
};

/// Holds the current checkpoint index for the runner to track progress.
#[derive(Default)]
struct CheckpointRunnerState {
    pub current_checkpoint_idx: Option<u64>,
}

/// Periodically polls for the latest checkpoint index and updates the current index.
/// Dispatches tasks when a new checkpoint is detected.
pub(crate) async fn checkpoint_proof_runner(
    operator: CheckpointOperator,
    poll_interval_s: u64,
    task_tracker: Arc<Mutex<TaskTracker>>,
    db: Arc<ProofDb>,
) {
    info!(?poll_interval_s, "Checkpoint runner started");
    let mut ticker = interval(Duration::from_secs(poll_interval_s));
    let mut runner_state = CheckpointRunnerState::default();

    loop {
        ticker.tick().await;

        if let Err(e) = process_checkpoint(&operator, &task_tracker, &db, &mut runner_state).await {
            error!(err = ?e, "error processing checkpoint");
        }
    }
}

async fn process_checkpoint(
    operator: &CheckpointOperator,
    task_tracker: &Arc<Mutex<TaskTracker>>,
    db: &Arc<ProofDb>,
    runner_state: &mut CheckpointRunnerState,
) -> anyhow::Result<()> {
    let res = fetch_latest_checkpoint_index(operator.cl_client()).await;
    let fetched_ckpt = match res {
        Ok(idx) => idx,
        Err(e) => {
            warn!(err = %e, "unable to fetch latest checkpoint index");
            return Ok(());
        }
    };

    let cur = runner_state.current_checkpoint_idx;
    if !should_update_checkpoint(cur, fetched_ckpt) {
        info!(fetched = %fetched_ckpt, ?cur, "fetched checkpoint is not newer than current");
        return Ok(());
    }

    operator
        .create_task(fetched_ckpt, task_tracker.clone(), db)
        .await?;
    runner_state.current_checkpoint_idx = Some(fetched_ckpt);

    Ok(())
}

fn should_update_checkpoint(current: Option<u64>, new: u64) -> bool {
    current.is_none_or(|current| new > current)
}
