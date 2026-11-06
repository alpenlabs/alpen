//! Integrated prover service for checkpoint proof generation.
//!
//! Provides an in-process prover that generates checkpoint validity proofs
//! using the paas framework. Reads OL data directly from local storage.
//!
//! Gated behind the `prover` feature flag and activated when a `[prover]`
//! section is present in the config.

mod context;
mod errors;
mod receipt_hook;
mod runner;
mod spec;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use strata_config::{ProverBackend, ProverConfig};
use strata_ol_checkpoint::ProofNotify;
#[cfg(feature = "sp1")]
use strata_ol_checkpoint_artifacts::sp1::load_checkpoint_registry;
use strata_ol_checkpoint_artifacts::{
    CheckpointArtifactRegistry, LoadedCheckpointPredicates, native_checkpoint_registry,
    startup::check_startup_artifacts_blocking,
};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::OLSpecId;
use strata_paas::{Prover, ProverBuilder, ProverServiceBuilder, RetryConfig};
use strata_storage::{NodeStorage, VersionedTaskStore};
use strata_tasks::TaskExecutor;
use tracing::info;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::SP1HostConfig;

use self::{
    context::CheckpointProverContextImpl, receipt_hook::CheckpointReceiptHook,
    runner::CheckpointRunner, spec::CheckpointSpec,
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
/// Launches a fixed-host paas service for each supported proving spec and spawns a
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

    let runtime_params = runctx.ol_params().runtime_params();

    // Each proving spec owns a fixed host and sees only its own task records.
    // V0 artifacts are retained for the startup key check but have no proving service.
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

    // TODO(STR-4610): Handle persisted permanently failed checkpoint tasks before
    // launching prover services; otherwise they can keep an epoch blocked across restarts.

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
    info!(backend = ?prover_config.backend, specs = handles.len(), "checkpoint prover services started");

    let context = CheckpointProverContextImpl::new(storage, handles);
    let sync_rx = runctx.status_channel().subscribe_ol_sync();
    executor.spawn_critical_async("checkpoint-proof-runner", async move {
        CheckpointRunner::new(context).run(sync_rx).await
    });

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
        .into_proving_hosts()
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

#[cfg(test)]
#[path = "tests.rs"]
mod integration_tests;
