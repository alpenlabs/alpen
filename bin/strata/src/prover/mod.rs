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

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use strata_config::{ProverBackend, ProverConfig};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_checkpoint::ProofNotify;
#[cfg(feature = "sp1")]
use strata_ol_checkpoint_artifacts::sp1::load_checkpoint_registry;
use strata_ol_checkpoint_artifacts::{
    CheckpointArtifactRegistry, LoadedCheckpointPredicates, native_checkpoint_registry,
    startup::check_startup_artifacts_blocking,
};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::OLSpecId;
use strata_paas::{
    Prover, ProverBuilder, ProverHandle, ProverServiceBuilder, RetryConfig, TaskResult,
};
use strata_storage::{CheckpointProofDbManager, NodeStorage, VersionedTaskStore};
use strata_tasks::TaskExecutor;
use tokio::{sync::watch, task::spawn_blocking, time};
use tracing::{debug, error, info, warn};
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::SP1HostConfig;

use self::{
    errors::ProverError,
    receipt_hook::CheckpointReceiptHook,
    spec::{CheckpointSpec, CheckpointTask, checkpoint_task_spec},
};
use crate::run_context::RunContext;

/// Interval used to retry failed proof tasks even when no new epoch notification
/// is emitted.
// TODO(STR-3064): make this configurable via ProverConfig.retry_interval.
const PROVER_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Default end-to-end deadline applied to the SP1 prover network when
/// `ProverConfig::sp1_proof_deadline_secs` is not set. Chosen to comfortably
/// cover checkpoint proofs while still failing fast on stuck requests.
#[cfg(feature = "sp1")]
const DEFAULT_SP1_DEADLINE_SECS: u64 = 4 * 60 * 60;

/// Returns the checkpoint SP1 proof deadline configured for this node.
#[cfg(feature = "sp1")]
pub(crate) fn checkpoint_sp1_proof_deadline_secs(prover_config: &ProverConfig) -> u64 {
    prover_config
        .sp1_proof_deadline_secs
        .unwrap_or(DEFAULT_SP1_DEADLINE_SECS)
}

/// Builds the SP1 host config used by the checkpoint prover.
#[cfg(feature = "sp1")]
pub(crate) fn checkpoint_sp1_host_config(prover_config: &ProverConfig) -> SP1HostConfig {
    SP1HostConfig::default().with_deadline(Duration::from_secs(checkpoint_sp1_proof_deadline_secs(
        prover_config,
    )))
}

/// Starts the integrated prover service.
///
/// Launches a fixed-host paas service for every resident spec and spawns a
/// background runner that routes checkpoints by the current spec in their terminal OL state.
///
/// The caller must ensure that `config.prover` is `Some` before calling.
/// `proof_notify` is shared with the checkpoint worker — the receipt hook
/// signals it after writing a proof so the checkpoint worker wakes
/// immediately.
pub(crate) fn start_prover_service(
    runctx: &RunContext,
    executor: &Arc<TaskExecutor>,
    proof_notify: Arc<ProofNotify>,
) -> Result<()> {
    let prover_config: ProverConfig = runctx
        .config()
        .prover
        .clone()
        .expect("[prover] config section required when prover is enabled");

    validate_prover_config(&prover_config)?;

    let storage = runctx.storage().clone();
    let proof_db = storage.checkpoint_proof().clone();

    let runtime_params = runctx.ol_params().runtime_params();

    // Each resident spec owns a fixed host and sees only its own task records.
    let (provers, predicates) = match prover_config.backend {
        ProverBackend::Native => build_checkpoint_provers(
            native_checkpoint_registry(runtime_params),
            &storage,
            runtime_params,
            &proof_notify,
            |builder, host| builder.native(host),
        ),
        #[cfg(feature = "sp1")]
        ProverBackend::Sp1 => {
            let registry = runctx
                .task_manager
                .handle()
                .block_on(load_checkpoint_registry(
                    &prover_config.artifacts,
                    runtime_params,
                    checkpoint_sp1_host_config(&prover_config),
                ))?;
            build_checkpoint_provers(
                registry,
                &storage,
                runtime_params,
                &proof_notify,
                |builder, host| builder.remote(host),
            )
        }
        #[cfg(not(feature = "sp1"))]
        ProverBackend::Sp1 => {
            unreachable!("SP1 feature checked by validate_prover_config")
        }
    };

    // Require artifacts for the locally canonical checkpoint state before any service
    // starts recovering tasks. L1/ASM catch-up continues independently in the background.
    let block = check_startup_artifacts_blocking(
        &storage,
        &predicates,
        runctx.ol_params().genesis_l1_block(),
    )
    .context("checkpoint artifact check failed at startup")?;
    info!(%block, "checkpoint artifacts checked at startup");

    // Each scanner receives a spec-scoped store, including startup recovery.
    let mut handles = BTreeMap::new();
    for (spec, prover) in provers {
        let handle = runctx
            .task_manager
            .handle()
            .block_on(
                ProverServiceBuilder::new(prover)
                    .tick_interval(PROVER_RETRY_INTERVAL)
                    .launch(executor.as_ref()),
            )
            .with_context(|| format!("failed to launch checkpoint prover for {spec:?}"))?;
        handles.insert(spec, handle);
    }
    let provers = CheckpointProvers { handles };

    info!(backend = ?prover_config.backend, specs = provers.handles.len(), "checkpoint prover services started");

    // Resume from the last epoch that already has a checkpoint payload,
    // so we don't re-check every epoch from 1 on restart.
    let last_payload_epoch = runctx
        .storage()
        .ol_checkpoint()
        .get_last_checkpoint_payload_epoch_blocking()
        .ok()
        .flatten()
        .map(|c| c.epoch());
    let history_base_epoch = runctx
        .storage()
        .ol_block()
        .get_history_base_blocking()
        .ok()
        .flatten()
        .map(|commitment| commitment.epoch());

    // Spawn checkpoint proof runner
    let chain_worker_handle = runctx.chain_worker_handle();
    let epoch_rx = chain_worker_handle.subscribe_epoch_summaries();
    spawn_checkpoint_runner(
        executor,
        provers,
        epoch_rx,
        proof_db,
        runctx.storage().clone(),
        last_payload_epoch,
        history_base_epoch,
    );

    Ok(())
}

fn validate_prover_config(config: &ProverConfig) -> Result<()> {
    if matches!(config.backend, ProverBackend::Native) {
        anyhow::ensure!(
            config.artifacts.is_empty(),
            "[prover.artifacts] bundles require the SP1 backend"
        );
    }

    #[cfg(not(feature = "sp1"))]
    if matches!(config.backend, ProverBackend::Sp1) {
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
/// epoch, avoiding redundant DB lookups for already-completed epochs. The
/// history base floors the cursor so a promoted sequencer never proves an
/// anchored epoch whose local block bodies are unavailable.
// TODO(STR-3064): split this into smaller helpers.
fn spawn_checkpoint_runner(
    executor: &TaskExecutor,
    provers: CheckpointProvers,
    mut epoch_rx: watch::Receiver<Option<EpochCommitment>>,
    proof_db: Arc<CheckpointProofDbManager>,
    storage: Arc<NodeStorage>,
    last_payload_epoch: Option<Epoch>,
    history_base_epoch: Option<Epoch>,
) {
    executor.spawn_critical_async("checkpoint-proof-runner", async move {
        // Resume after the last checkpointed or anchored epoch, or start from epoch 1.
        let mut next_epoch_to_prove =
            derive_next_epoch_to_prove(last_payload_epoch, history_base_epoch);
        // The epoch-summary watch channel resets to `None` on restart, so fall
        // back to the last summarized epoch from storage. Otherwise the catch-up
        // loop below would idle at 0 and never re-prove epochs whose proofs were
        // cleared by startup reconciliation until a new terminal epoch arrives.
        let mut latest_epoch = epoch_rx
            .borrow()
            .map(|commitment| commitment.epoch())
            .or_else(|| {
                storage
                    .ol_checkpoint()
                    .get_last_summarized_epoch_blocking()
                    .ok()
                    .flatten()
            })
            .unwrap_or(0);
        // Keep configuration alerts actionable without repeating them on every tick.
        let mut waiting: Option<(EpochCommitment, String)> = None;
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

                let task = CheckpointTask(commitment);
                let input_storage = Arc::clone(&storage);
                let required_spec = match spawn_blocking(move || {
                    checkpoint_task_spec(&input_storage, task)
                }).await {
                    Ok(result) => result,
                    Err(error) => {
                        warn!(%epoch, %error, "checkpoint routing task failed, will retry");
                        break;
                    }
                };
                let routing = required_spec.map_err(CheckpointRoutingError::State).and_then(|spec| {
                    provers.for_spec(spec).ok_or(CheckpointRoutingError::MissingArtifact { spec })
                });
                let prover_handle = match routing {
                    Ok(handle) => handle,
                    Err(failure) => {
                        let reason = failure.to_string();
                        if !waiting.as_ref().is_some_and(|(previous_task, previous_reason)| {
                            *previous_task == commitment && previous_reason == &reason
                        }) {
                            if matches!(&failure,
                                CheckpointRoutingError::MissingArtifact { .. }
                            ) {
                                error!(%epoch, %failure, "checkpoint proof awaits operator configuration");
                            } else if matches!(&failure,
                                CheckpointRoutingError::State(
                                    ProverError::EpochCommitmentNotFound(_)
                                    | ProverError::EpochTerminalStateNotFound { .. }
                                )
                            ) {
                                debug!(%epoch, %failure, "checkpoint routing awaits state, will retry");
                            } else {
                                warn!(%epoch, %failure, "checkpoint routing failed, will retry");
                            }
                            waiting = Some((commitment, reason));
                        }
                        break;
                    }
                };
                waiting = None;
                info!(%epoch, "submitting checkpoint proof task");
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

/// Distinguishes unavailable epoch state from unavailable proving artifacts.
#[derive(Debug, thiserror::Error)]
enum CheckpointRoutingError {
    #[error(transparent)]
    State(ProverError),
    #[error("no checkpoint artifact is configured for OL spec {spec:?}")]
    MissingArtifact { spec: OLSpecId },
}

/// Holds fixed-host services keyed by the spec recorded in each checkpoint's terminal state.
struct CheckpointProvers {
    handles: BTreeMap<OLSpecId, ProverHandle<CheckpointSpec>>,
}

impl CheckpointProvers {
    fn for_spec(&self, spec: OLSpecId) -> Option<&ProverHandle<CheckpointSpec>> {
        self.handles.get(&spec)
    }
}

/// Builds each service over a store scoped to its fixed program's spec.
fn build_checkpoint_provers<H>(
    registry: CheckpointArtifactRegistry<H>,
    storage: &Arc<NodeStorage>,
    runtime_params: OLRuntimeParams,
    proof_notify: &Arc<ProofNotify>,
    build: impl Fn(ProverBuilder<CheckpointSpec>, H) -> Prover<CheckpointSpec>,
) -> (
    BTreeMap<OLSpecId, Prover<CheckpointSpec>>,
    LoadedCheckpointPredicates,
) {
    let predicates = registry.to_predicates();
    let provers = registry
        .into_hosts()
        .map(|(spec, host)| {
            let builder = ProverBuilder::new(CheckpointSpec::new(
                Arc::clone(storage),
                runtime_params,
                spec,
            ))
            .task_store(VersionedTaskStore::new(
                storage.prover_tasks().clone(),
                spec,
            ))
            .receipt_hook(CheckpointReceiptHook::new(
                storage.checkpoint_proof().clone(),
                Arc::clone(proof_notify),
            ))
            .retry(RetryConfig::default());
            (spec, build(builder, host))
        })
        .collect();
    (provers, predicates)
}

fn derive_next_epoch_to_prove(
    last_payload_epoch: Option<Epoch>,
    history_base_epoch: Option<Epoch>,
) -> Epoch {
    last_payload_epoch
        .max(history_base_epoch)
        .map_or(1, |epoch| epoch + 1)
}

#[cfg(test)]
mod tests {
    use super::derive_next_epoch_to_prove;

    #[test]
    fn next_epoch_without_history_base_preserves_existing_behavior() {
        assert_eq!(derive_next_epoch_to_prove(None, None), 1);
        assert_eq!(derive_next_epoch_to_prove(Some(4), None), 5);
    }

    #[test]
    fn next_epoch_with_history_base_and_no_payload_starts_after_floor() {
        assert_eq!(derive_next_epoch_to_prove(None, Some(5)), 6);
        assert_eq!(derive_next_epoch_to_prove(Some(4), Some(5)), 6);
    }

    #[test]
    fn next_epoch_with_payload_after_history_base_starts_after_payload() {
        assert_eq!(derive_next_epoch_to_prove(Some(6), Some(5)), 7);
    }

    #[test]
    fn next_epoch_after_payload_deletion_returns_to_history_base_floor() {
        assert_eq!(derive_next_epoch_to_prove(None, Some(5)), 6);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod integration_tests;
