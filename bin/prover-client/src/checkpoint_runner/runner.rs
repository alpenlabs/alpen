use std::{future::Future, pin::Pin, sync::Arc};

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
    let res = fetch_next_unproven_checkpoint_index(operator.ol_client()).await;
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
/// and submit the proof to OL client
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
        .map_err(|e| anyhow::anyhow!("DB error: {e}"))?
        .is_some()
    {
        info!(%checkpoint_idx, "Checkpoint proof already exists, submitting to OL");

        // Proof exists, submit it to OL
        operator
            .submit_checkpoint_proof(checkpoint_idx, &proof_key, db)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit checkpoint to OL: {e}"))?;

        info!(%checkpoint_idx, "Checkpoint proof submitted to OL");
        return Ok(());
    }

    // Create checkpoint dependencies (OL Stf)
    let ol_stf_deps = operator
        .create_checkpoint_deps(checkpoint_idx, db)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create checkpoint dependencies: {e}"))?;

    // For each OLStf dependency, create EvmEeStf dependencies and submit recursively
    for dep_ctx in &ol_stf_deps {
        if let ProofContext::OLStf(start, end) = dep_ctx {
            // Create EvmEeStf dependencies for this OLStf
            operator
                .ol_stf_operator()
                .create_ol_stf_deps(*start, *end, db)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create OL stf dependencies: {e}"))?;

            // Submit OLStf and its EvmEeStf dependencies recursively
            submit_proof_context_recursive(*dep_ctx, prover_handle, db).await?;
        }
    }

    // Execute checkpoint task and await completion (NO POLLING!)
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

    info!(%checkpoint_idx, "Checkpoint proof completed, submitting to OL");

    // Submit checkpoint proof to OL client
    operator
        .submit_checkpoint_proof(checkpoint_idx, &proof_key, db)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit checkpoint to OL: {e}"))?;

    info!(%checkpoint_idx, "Checkpoint proof submitted to OL");
    Ok(())
}

/// Recursively submit a proof context and all its dependencies
fn submit_proof_context_recursive<'a>(
    proof_ctx: ProofContext,
    prover_handle: &'a ProverHandle<ProofTask>,
    db: &'a Arc<ProofDBSled>,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + 'a + Send>> {
    Box::pin(async move {
        let proof_key = proof_key_for(proof_ctx);

        // Check if proof already exists
        if db
            .get_proof(&proof_key)
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
            .is_some()
        {
            return Ok(());
        }

        // Get dependencies from database
        let proof_deps = db
            .get_proof_deps(proof_ctx)
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
            .unwrap_or_default();

        // Submit dependency tasks recursively
        for dep_ctx in &proof_deps {
            submit_proof_context_recursive(*dep_ctx, prover_handle, db).await?;
        }

        // Execute main task and await completion
        info!(?proof_ctx, "Executing dependency proof task");

        let result = prover_handle
            .execute_task(ProofTask(proof_ctx), zkvm_backend())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute task: {}", e))?;

        // Check result
        match result {
            TaskResult::Completed { uuid } => {
                info!(?proof_ctx, %uuid, "Dependency proof completed successfully");
                Ok(())
            }
            TaskResult::Failed { uuid, error } => Err(anyhow::anyhow!(
                "Dependency proof failed (UUID: {}, ctx: {:?}): {}",
                uuid,
                proof_ctx,
                error
            )),
        }
    })
}

fn should_update_checkpoint(current: Option<u64>, new: u64) -> bool {
    current.is_none_or(|current| new > current)
}
