//! Integrated prover service for checkpoint proof generation.
//!
//! Provides an in-process prover that generates checkpoint validity proofs
//! using the paas framework. Reads OL data directly from local storage.
//!
//! Gated behind the `prover` feature flag and activated when a `[prover]`
//! section is present in the config.

mod errors;
mod host_resolver;
mod input_fetcher;
mod proof_storer;
mod task;
mod task_store;

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use strata_config::{ProverBackend, ProverConfig};
use strata_identifiers::Epoch;
use strata_paas::{
    ProverHandle, ProverServiceBuilder, ProverServiceConfig, RemoteProofHandler, RetryConfig,
    TaskResult, ZkVmBackend,
};
use strata_primitives::{
    epoch::EpochCommitment,
    proof::{ProofContext, ProofKey},
};
use strata_proofimpl_checkpoint_new::program::CheckpointProgram;
use strata_storage::ProofDbManager;
use strata_tasks::TaskExecutor;
use tokio::{sync::watch, time};
use tracing::{debug, info, warn};

use self::{
    host_resolver::{CheckpointHostResolver, backend_from_config},
    input_fetcher::CheckpointInputFetcher,
    proof_storer::CheckpointProofStorer,
    task::{CheckpointTask, CheckpointVariant},
    task_store::PersistentTaskStore,
};
use crate::run_context::RunContext;

/// Interval used to retry failed proof tasks even when no new epoch notification
/// is emitted.
// TODO(STR-3064): make this configurable via ProverConfig.retry_interval.
const PROVER_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Type alias for the checkpoint proof handler.
type CheckpointHandler = RemoteProofHandler<
    CheckpointTask,
    CheckpointInputFetcher,
    CheckpointProofStorer,
    CheckpointHostResolver,
    CheckpointProgram,
>;

/// Starts the integrated prover service.
///
/// Launches a paas prover service with a checkpoint handler and spawns
/// a background runner that automatically proves new epochs as they complete.
///
/// The caller must ensure that `config.prover` is `Some` before calling.
/// `proof_notify` is shared with the checkpoint worker — the proof storer
/// signals it after writing a proof so the checkpoint worker wakes immediately.
pub(crate) fn start_prover_service(
    runctx: &RunContext,
    executor: &Arc<TaskExecutor>,
    proof_notify: Arc<strata_ol_checkpoint::ProofNotify>,
) -> Result<()> {
    let prover_config: ProverConfig = runctx
        .config()
        .prover
        .clone()
        .expect("[prover] config section required when prover is enabled");

    validate_backend_config(prover_config.backend)?;
    let backend = backend_from_config(prover_config.backend);

    let storage = runctx.storage().clone();
    let proof_db = runctx.storage().proof().clone();
    let prover_task_db = runctx.storage().prover_tasks().clone();

    // Create paas components.
    let fetcher = CheckpointInputFetcher::new(storage);
    let storer = CheckpointProofStorer::new(proof_db.clone(), proof_notify);
    let resolver = CheckpointHostResolver;
    let handler: Arc<CheckpointHandler> = Arc::new(RemoteProofHandler::new(
        fetcher,
        storer,
        resolver,
        executor.as_ref().clone(),
    ));

    // Explicitly zero all backends, then enable only the selected one.
    // This is required because the paas ProverServiceBuilder defaults
    // missing backends to 1 worker (unwrap_or(1) in builder.rs).
    // TODO(STR-1947): ProverServiceConfig should default to 0 workers for
    // unspecified backends, so callers only need to set the ones they want.
    let worker_counts = HashMap::from([
        (ZkVmBackend::Native, 0),
        (ZkVmBackend::SP1, 0),
        (ZkVmBackend::Risc0, 0),
        (backend.clone(), prover_config.workers),
    ]);

    let service_config = ProverServiceConfig::new(worker_counts);

    // Build and launch prover service
    let task_store = PersistentTaskStore::new(prover_task_db);
    let handle: ProverHandle<CheckpointTask> = runctx
        .task_manager
        .handle()
        .block_on(
            ProverServiceBuilder::<CheckpointTask>::new(service_config)
                .with_task_store(task_store)
                .with_retry_config(RetryConfig::default())
                .with_handler(CheckpointVariant::Checkpoint, handler)
                .launch(executor),
        )
        .context("failed to launch prover service")?;

    info!(?backend, "prover service started");

    // Resume from the last epoch that already has a checkpoint payload,
    // so we don't re-check every epoch from 1 on restart.
    let last_payload_epoch = runctx
        .storage()
        .ol_checkpoint()
        .get_last_checkpoint_payload_epoch_blocking()
        .ok()
        .flatten()
        .map(|c| c.epoch());

    // Spawn checkpoint proof runner
    let chain_worker_handle = runctx.chain_worker_handle();
    let epoch_rx = chain_worker_handle.subscribe_epoch_summaries();
    spawn_checkpoint_runner(
        executor,
        handle,
        epoch_rx,
        backend,
        proof_db,
        runctx.storage().clone(),
        last_payload_epoch,
    );

    Ok(())
}

fn validate_backend_config(backend: ProverBackend) -> Result<()> {
    #[cfg(feature = "sp1")]
    let _ = backend;

    #[cfg(not(feature = "sp1"))]
    if matches!(backend, ProverBackend::Sp1) {
        anyhow::bail!(
            "config.prover.backend=sp1 requires building `strata` with the `sp1` feature"
        );
    }

    Ok(())
}

/// Spawns a background task that watches for new epoch completions and
/// submits proof tasks for each new epoch.
///
/// Uses a cursor (`next_epoch_to_prove`) instead of tracking only the latest
/// epoch. This ensures no epochs are skipped if multiple complete while a
/// proof is in progress: when the current proof finishes, the runner catches
/// up through all missed epochs sequentially.
///
/// `last_payload_epoch` is the last epoch for which a checkpoint payload was
/// already built (read from DB at startup). The runner resumes from the next
/// epoch, avoiding redundant DB lookups for already-completed epochs.
// TODO(STR-3064): split this into smaller helpers.
fn spawn_checkpoint_runner(
    executor: &TaskExecutor,
    prover_handle: ProverHandle<CheckpointTask>,
    mut epoch_rx: watch::Receiver<Option<EpochCommitment>>,
    backend: ZkVmBackend,
    proof_db: Arc<ProofDbManager>,
    storage: Arc<strata_storage::NodeStorage>,
    last_payload_epoch: Option<Epoch>,
) {
    executor.spawn_critical_async("checkpoint-proof-runner", async move {
        // Resume after the last checkpointed epoch, or start from epoch 1.
        let mut next_epoch_to_prove: Epoch = last_payload_epoch.map_or(1, |e| e + 1);
        let mut latest_epoch = epoch_rx.borrow().map_or(0, |commitment| commitment.epoch());
        let mut retry_tick = time::interval(PROVER_RETRY_INTERVAL);
        retry_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                changed = epoch_rx.changed() => {
                    if changed.is_err() {
                        debug!("epoch summary channel closed, stopping checkpoint proof runner");
                        break;
                    }

                    if let Some(commitment) = *epoch_rx.borrow() {
                        latest_epoch = latest_epoch.max(commitment.epoch());
                        // Handle same-epoch reorgs (or any canonical commitment
                        // change for an already-processed epoch) by rewinding the
                        // cursor so we re-evaluate proof presence for that epoch.
                        let rewind_epoch = commitment.epoch().max(1);
                        if rewind_epoch < next_epoch_to_prove {
                            debug!(
                                rewind_epoch,
                                old_next_epoch = next_epoch_to_prove,
                                "observed commitment update for already-processed epoch; rewinding prover cursor"
                            );
                            next_epoch_to_prove = rewind_epoch;
                        }
                    }
                }
                _ = retry_tick.tick() => {}
            }

            // Catch up on all epochs from cursor to latest.
            while next_epoch_to_prove <= latest_epoch {
                let epoch = next_epoch_to_prove;

                // Resolve the full epoch commitment from the checkpoint DB.
                let commitment = match storage
                    .ol_checkpoint()
                    .get_canonical_epoch_commitment_at_blocking(epoch)
                {
                    Ok(Some(c)) => c,
                    Ok(None) => {
                        debug!(%epoch, "epoch commitment not yet available, will retry");
                        break;
                    }
                    Err(e) => {
                        warn!(%epoch, %e, "failed to read epoch commitment, will retry");
                        break;
                    }
                };

                // Skip if proof already exists (idempotency after restart).
                let task = CheckpointTask::new(commitment, backend.clone());
                let zkvm = task.proof_zkvm().map_err(|e| {
                    anyhow::anyhow!(
                        "unsupported checkpoint backend at epoch {epoch}: backend={:?}, error={e}",
                        task.backend
                    )
                })?;
                let proof_key = ProofKey::new(ProofContext::CheckpointCommitment(commitment), zkvm);
                if proof_db.get_proof(proof_key).ok().flatten().is_some() {
                    debug!(%epoch, "proof already exists, skipping");
                    next_epoch_to_prove += 1;
                    continue;
                }

                info!(%epoch, "submitting checkpoint proof task");

                match prover_handle.execute_task(task, backend.clone()).await {
                    Ok(TaskResult::Completed { uuid }) => {
                        // Re-check canonical commitment before advancing the
                        // cursor. If the epoch reorged while proving, keep the
                        // cursor at this epoch so we immediately reprove.
                        let latest_commitment = match storage
                            .ol_checkpoint()
                            .get_canonical_epoch_commitment_at_blocking(epoch)
                        {
                            Ok(Some(c)) => c,
                            Ok(None) => {
                                warn!(
                                    %epoch,
                                    %uuid,
                                    "canonical commitment missing after proof completion; will retry epoch"
                                );
                                break;
                            }
                            Err(e) => {
                                warn!(
                                    %epoch,
                                    %uuid,
                                    %e,
                                    "failed to re-check canonical commitment after proof completion; will retry epoch"
                                );
                                break;
                            }
                        };
                        if latest_commitment != commitment {
                            warn!(
                                %epoch,
                                %uuid,
                                proved_commitment = ?commitment,
                                canonical_commitment = ?latest_commitment,
                                "epoch commitment changed while proving; reproving epoch"
                            );
                            continue;
                        }

                        info!(%epoch, %uuid, "checkpoint proof completed");
                        next_epoch_to_prove += 1;
                    }
                    Ok(TaskResult::Failed { uuid, error }) => {
                        warn!(%epoch, %uuid, %error, "checkpoint proof failed, will retry");
                        break;
                    }
                    Err(e) => {
                        warn!(%epoch, %e, "checkpoint proof failed, will retry");
                        break;
                    }
                }
            }
        }

        Ok(())
    });

    debug!("spawned checkpoint proof runner");
}
