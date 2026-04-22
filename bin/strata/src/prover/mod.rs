//! Integrated prover service for checkpoint proof generation.
//!
//! Provides an in-process prover that generates checkpoint validity proofs
//! using the paas framework. Reads OL data directly from local storage.
//!
//! Gated behind the `prover` feature flag and activated when a `[prover]`
//! section is present in the config.

mod errors;
mod receipt_hook;
mod spec;

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use strata_config::{ProverBackend, ProverConfig};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_paas::{ProverBuilder, ProverHandle, ProverServiceBuilder, RetryConfig, TaskResult};
use strata_proofimpl_checkpoint_new::program::CheckpointProgram;
use strata_storage::CheckpointProofDbManager;
use strata_tasks::TaskExecutor;
use tokio::{sync::watch, time};
use tracing::{debug, info, warn};

use self::{
    receipt_hook::CheckpointReceiptHook,
    spec::{CheckpointSpec, CheckpointTask},
};
use crate::run_context::RunContext;

/// Interval used to retry failed proof tasks even when no new epoch notification
/// is emitted.
// TODO(STR-3064): make this configurable via ProverConfig.retry_interval.
const PROVER_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Starts the integrated prover service.
///
/// Launches a paas prover service for checkpoint proofs and spawns a
/// background runner that submits proof tasks as new epochs complete.
///
/// The caller must ensure that `config.prover` is `Some` before calling.
/// `proof_notify` is shared with the checkpoint worker — the receipt hook
/// signals it after writing a proof so the checkpoint worker wakes
/// immediately.
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

    let storage = runctx.storage().clone();
    let proof_db = storage.checkpoint_proof().clone();

    // Build the spec + hook. The backend choice is fixed here at build
    // time rather than being part of task identity — the new paas erases
    // the host type inside the prove strategy.
    let spec = CheckpointSpec::new(storage.clone());
    let hook = CheckpointReceiptHook::new(proof_db.clone(), proof_notify);

    // Task store: the node's `ProverTaskDbManager` implements
    // `strata_paas::TaskStore` directly, so the manager *is* the persistent
    // task store — no extra adapter layer.
    let task_store = runctx.storage().prover_tasks().clone();

    // Pick native vs. remote strategy at build time.
    let prover = match prover_config.backend {
        ProverBackend::Native => ProverBuilder::new(spec)
            .task_store(task_store)
            .receipt_hook(hook)
            .retry(RetryConfig::default())
            .native(CheckpointProgram::native_host()),
        #[cfg(feature = "sp1")]
        ProverBackend::Sp1 => {
            use strata_zkvm_hosts::sp1::CHECKPOINT_NEW_HOST;
            // prover-core's `.remote(host)` takes the host by value and
            // re-wraps it in its own Arc inside RemoteStrategy. SP1Host
            // is Clone (only holds a SP1ProvingKey), so cloning from the
            // shared static is fine.
            let mut host: zkaleido_sp1_host::SP1Host = (**CHECKPOINT_NEW_HOST).clone();
            if let Some(secs) = prover_config.sp1_proof_deadline_secs {
                host = host.with_deadline(Duration::from_secs(secs));
                info!(deadline_secs = secs, "sp1 prover deadline configured");
            }
            ProverBuilder::new(spec)
                .task_store(task_store)
                .receipt_hook(hook)
                .retry(RetryConfig::default())
                .remote(host)
        }
        #[cfg(not(feature = "sp1"))]
        ProverBackend::Sp1 => {
            // validate_backend_config rejects this at startup.
            unreachable!(
                "SP1 backend requested but sp1 feature is not enabled; \
                 validate_backend_config should have caught this at startup"
            )
        }
    };

    // Launch the service. `tick_interval` drives both startup recovery
    // (re-spawning unfinished tasks) and the retry scanner.
    let handle: ProverHandle<CheckpointSpec> = runctx
        .task_manager
        .handle()
        .block_on(
            ProverServiceBuilder::new(prover)
                .tick_interval(PROVER_RETRY_INTERVAL)
                .launch(executor),
        )
        .context("failed to launch prover service")?;

    info!(backend = ?prover_config.backend, "prover service started");

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
    prover_handle: ProverHandle<CheckpointSpec>,
    mut epoch_rx: watch::Receiver<Option<EpochCommitment>>,
    proof_db: Arc<CheckpointProofDbManager>,
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
                if proof_db.get_proof(&commitment).ok().flatten().is_some() {
                    debug!(%epoch, "proof already exists, skipping");
                    next_epoch_to_prove += 1;
                    continue;
                }

                info!(%epoch, "submitting checkpoint proof task");

                let task = CheckpointTask(commitment);
                match prover_handle.execute(task).await {
                    Ok(TaskResult::Completed { task: _ }) => {
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
                                    "canonical commitment missing after proof completion; will retry epoch"
                                );
                                break;
                            }
                            Err(e) => {
                                warn!(
                                    %epoch,
                                    %e,
                                    "failed to re-check canonical commitment after proof completion; will retry epoch"
                                );
                                break;
                            }
                        };
                        if latest_commitment != commitment {
                            warn!(
                                %epoch,
                                proved_commitment = ?commitment,
                                canonical_commitment = ?latest_commitment,
                                "epoch commitment changed while proving; reproving epoch"
                            );
                            continue;
                        }

                        info!(%epoch, "checkpoint proof completed");
                        next_epoch_to_prove += 1;
                    }
                    Ok(TaskResult::Failed { task: _, error }) => {
                        warn!(%epoch, %error, "checkpoint proof failed, will retry");
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
