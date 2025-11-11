use std::sync::Arc;

use strata_db_types::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{ProverHandle, ZkVmBackend, ZkVmTaskId};
use strata_primitives::proof::{ProofContext, ProofKey};
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::{
    checkpoint_runner::fetch::fetch_next_unproven_checkpoint_index,
    operators::checkpoint::CheckpointOperator,
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
    prover_handle: ProverHandle<ProofContext>,
    db: Arc<ProofDBSled>,
) {
    info!(%poll_interval_s, "Checkpoint runner started");
    let mut ticker = interval(Duration::from_secs(poll_interval_s));
    let mut runner_state = CheckpointRunnerState::default();

    loop {
        ticker.tick().await;

        if let Err(e) = process_checkpoint(&operator, &prover_handle, &db, &mut runner_state).await {
            error!(err = ?e, "error processing checkpoint");
        }
    }
}

async fn process_checkpoint(
    operator: &CheckpointOperator,
    prover_handle: &ProverHandle<ProofContext>,
    db: &Arc<ProofDBSled>,
    runner_state: &mut CheckpointRunnerState,
) -> anyhow::Result<()> {
    let res = fetch_next_unproven_checkpoint_index(operator.cl_client()).await;
    let fetched_ckpt = match res {
        Ok(Some(idx)) => idx,
        Ok(None) => {
            info!("no unproven checkpoints available");
            return Ok(());
        }
        Err(e) => {
            warn!(err = %e, "unable to fetch next unproven checkpoint index");
            return Ok(());
        }
    };

    let cur = runner_state.current_checkpoint_idx;
    if !should_update_checkpoint(cur, fetched_ckpt) {
        info!(fetched = %fetched_ckpt, ?cur, "fetched checkpoint is not newer than current");
        return Ok(());
    }

    // Submit checkpoint task using PaaS
    submit_checkpoint_task(fetched_ckpt, prover_handle, db).await?;
    runner_state.current_checkpoint_idx = Some(fetched_ckpt);

    Ok(())
}

/// Submit a checkpoint task to PaaS, handling dependencies
async fn submit_checkpoint_task(
    checkpoint_idx: u64,
    prover_handle: &ProverHandle<ProofContext>,
    db: &Arc<ProofDBSled>,
) -> anyhow::Result<()> {
    let proof_ctx = ProofContext::Checkpoint(checkpoint_idx);

    // Determine backend based on features
    let backend = get_backend();
    let zkvm = match backend {
        ZkVmBackend::SP1 => strata_primitives::proof::ProofZkVm::SP1,
        ZkVmBackend::Native => strata_primitives::proof::ProofZkVm::Native,
        ZkVmBackend::Risc0 => anyhow::bail!("Risc0 not supported"),
    };

    let proof_key = ProofKey::new(proof_ctx, zkvm);

    // Check if proof already exists
    if db.get_proof(&proof_key)
        .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
        .is_some()
    {
        info!(%checkpoint_idx, "Checkpoint proof already exists");
        return Ok(());
    }

    // Get or create proof dependencies
    let proof_deps = db
        .get_proof_deps(proof_ctx)
        .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
        .unwrap_or_default();

    // Submit dependency tasks first
    for dep_ctx in &proof_deps {
        let dep_proof_key = ProofKey::new(*dep_ctx, zkvm);

        // Check if dependency proof already exists
        if db.get_proof(&dep_proof_key)
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
            .is_some()
        {
            continue;
        }

        // Submit dependency task to PaaS
        let dep_task_id = ZkVmTaskId {
            program: *dep_ctx,
            backend: backend.clone(),
        };

        prover_handle
            .submit_task(dep_task_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit dependency task: {}", e))?;
    }

    // Submit main checkpoint task
    let task_id = ZkVmTaskId {
        program: proof_ctx,
        backend,
    };

    prover_handle
        .submit_task(task_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit checkpoint task: {}", e))?;

    info!(%checkpoint_idx, "Submitted checkpoint proof task");
    Ok(())
}

/// Helper to determine backend based on features
fn get_backend() -> ZkVmBackend {
    #[cfg(feature = "sp1")]
    {
        ZkVmBackend::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ZkVmBackend::Native
    }
}

fn should_update_checkpoint(current: Option<u64>, new: u64) -> bool {
    current.is_none_or(|current| new > current)
}
