use std::sync::Arc;

use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_paas::{ProverHandle, TaskResult};
use strata_primitives::proof::ProofContext;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::{
    checkpoint_runner::fetch::fetch_next_unproven_checkpoint_index,
    operators::checkpoint::CheckpointOperator,
    service::{proof_key_for, zkvm_backend, ProofTask},
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
    prover_handle: ProverHandle<ProofTask>,
    db: Arc<ProofDBSled>,
) {
    info!(%poll_interval_s, "Checkpoint runner started");
    let mut ticker = interval(Duration::from_secs(poll_interval_s));
    let mut runner_state = CheckpointRunnerState::default();

    loop {
        ticker.tick().await;

        if let Err(e) = process_checkpoint(&operator, &prover_handle, &db, &mut runner_state).await
        {
            error!(err = ?e, "error processing checkpoint");
        }
    }
}

async fn process_checkpoint(
    operator: &CheckpointOperator,
    prover_handle: &ProverHandle<ProofTask>,
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

    // Submit checkpoint task using Prover Service
    submit_checkpoint_task(fetched_ckpt, operator, prover_handle, db).await?;
    runner_state.current_checkpoint_idx = Some(fetched_ckpt);

    Ok(())
}

/// Submit a checkpoint task to Prover Service, wait for completion,
/// and submit the proof to CL client
async fn submit_checkpoint_task(
    checkpoint_idx: u64,
    operator: &CheckpointOperator,
    prover_handle: &ProverHandle<ProofTask>,
    db: &Arc<ProofDBSled>,
) -> anyhow::Result<()> {
    let proof_ctx = ProofContext::Checkpoint(checkpoint_idx);
    let proof_key = proof_key_for(proof_ctx);

    // Check if proof already exists
    if db
        .get_proof(&proof_key)
        .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
        .is_some()
    {
        info!(%checkpoint_idx, "Checkpoint proof already exists, submitting to CL");

        // Proof exists, submit it to CL
        operator
            .submit_checkpoint_proof(checkpoint_idx, &proof_key, db)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit checkpoint to CL: {}", e))?;

        info!(%checkpoint_idx, "Checkpoint proof submitted to CL");
        return Ok(());
    }

    // Execute checkpoint task and await completion
    info!(%checkpoint_idx, "Executing checkpoint proof task");

    let result = prover_handle
        .execute_task(ProofTask(proof_ctx), zkvm_backend())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute checkpoint task: {}", e))?;

    // Check result
    match result {
        TaskResult::Completed { uuid } => {
            info!(%checkpoint_idx, %uuid, "Checkpoint proof completed successfully");
        }
        TaskResult::Failed { uuid, error } => {
            return Err(anyhow::anyhow!(
                "Checkpoint proof failed (UUID: {}): {}",
                uuid,
                error
            ));
        }
    }

    info!(%checkpoint_idx, "Checkpoint proof completed, submitting to CL");

    // Submit checkpoint proof to CL client
    operator
        .submit_checkpoint_proof(checkpoint_idx, &proof_key, db)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit checkpoint to CL: {}", e))?;

    info!(%checkpoint_idx, "Checkpoint proof submitted to CL");
    Ok(())
}

fn should_update_checkpoint(current: Option<u64>, new: u64) -> bool {
    current.is_none_or(|current| new > current)
}
