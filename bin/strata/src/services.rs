//! Service spawning and lifecycle management.

use std::sync::Arc;
#[cfg(feature = "sequencer")]
use std::time::{Duration, Instant};

use anyhow::Result;
#[cfg(feature = "sequencer")]
use strata_asm_worker::AsmWorkerHandle;
use strata_btcio::reader::query::{ReaderValidation, bitcoin_data_reader_task};
use strata_chain_worker::{ChainWorkerHandle, start_chain_worker_service_from_ctx};
use strata_consensus_logic::{
    AsmBlockSubmitter, SyncServiceHandle,
    sync_manager::{spawn_asm_worker_with_ctx, spawn_csm_listener_with_ctx},
};
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::NodeContext;
use strata_ol_checkpoint::OLCheckpointBuilder;
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};
use strata_service::ServiceMonitor;
#[cfg(feature = "sequencer")]
use tokio::time::{sleep, timeout};
#[cfg(feature = "sequencer")]
use tracing::warn;

#[cfg(feature = "sequencer")]
use crate::checkpoint_reconcile::reconcile_settled_checkpoint_queue;
use crate::{
    checkpoint_reconcile::reconcile_unaccepted_checkpoint_artifacts,
    context::ensure_genesis,
    css, fcm,
    helpers::build_btcio_params,
    run_context::{RunContext, ServiceHandles},
};

#[cfg(feature = "sequencer")]
mod sequencer_services {
    use std::{sync::Arc, time::Duration};

    use anyhow::{Result, anyhow};
    use strata_btcio::{
        broadcaster::{BroadcasterBuilder, L1BroadcastHandle},
        writer::{BundlerBuilder, EnvelopeHandle, WatcherBuilder, WriterContext},
    };
    use strata_config::EpochSealingConfig;
    use strata_db_types::backend::DatabaseBackend;
    use strata_node_context::NodeContext;
    use strata_ol_block_assembly::{
        BlockasmBuilder, BlockasmHandle, FixedSlotSealing, LimitAwareSealing, MempoolProviderImpl,
    };
    use strata_ol_checkpoint::AsmCheckpointInspector;
    use strata_ol_mempool::MempoolHandle;
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_service::DumbTickHandle;
    use strata_storage::{BroadcastDbOps, ops::writer::EnvelopeDataOps};
    use tokio::sync::mpsc;

    use crate::{
        checkpoint_auth::CheckpointSequencerKeyProvider,
        checkpoint_reconcile::CheckpointFailureCleanup,
        helpers::generate_sequencer_address,
        run_context::{SequencerServiceHandles, ServiceHandlesBuilder},
    };

    pub(super) fn start_if_enabled(
        nodectx: &NodeContext,
        mempool_handle: Option<Arc<MempoolHandle>>,
    ) -> Result<Option<SequencerServiceHandles>> {
        if !nodectx.config().client.is_sequencer {
            return Ok(None);
        }

        // Sequencer mode always starts the mempool upstream; if not, that's a
        // wiring regression in `start_strata_services`.
        let mempool_handle = mempool_handle.expect("sequencer node must have a mempool handle");

        let broadcast_handle = Arc::new(start_broadcaster(nodectx)?);
        let (envelope_handle, watcher_handle) = start_writer(nodectx, broadcast_handle.clone())?;
        let blockasm_handle = Arc::new(start_block_assembly(nodectx, mempool_handle)?);

        Ok(Some(SequencerServiceHandles::new(
            broadcast_handle,
            envelope_handle,
            blockasm_handle,
            watcher_handle,
        )))
    }

    pub(super) fn attach_service_handles(
        builder: ServiceHandlesBuilder,
        sequencer_handles: Option<SequencerServiceHandles>,
    ) -> ServiceHandlesBuilder {
        builder.with_sequencer_handles(sequencer_handles)
    }

    /// Starts the L1 broadcaster task.
    ///
    /// Manages L1 transaction broadcasting and tracks confirmation status.
    fn start_broadcaster(nodectx: &NodeContext) -> Result<L1BroadcastHandle> {
        let broadcast_db = nodectx.storage().db().broadcast_db();
        let broadcast_ops = Arc::new(BroadcastDbOps::new(
            nodectx.storage().handle().clone(),
            broadcast_db,
        ));

        nodectx.task_manager().handle().block_on(async {
            BroadcasterBuilder::new(
                nodectx.bitcoin_client().clone(),
                broadcast_ops,
                super::build_btcio_params(
                    nodectx.asm_params(),
                    nodectx.config().btcio.l1_reorg_safe_depth,
                ),
            )
            .with_broadcast_poll_interval_ms(nodectx.config().btcio.broadcaster.poll_interval_ms)
            .launch(nodectx.executor().as_ref())
            .await
        })
    }

    /// Starts the L1 writer/envelope task.
    ///
    /// Bundles L1 intents, creates envelope transactions, and publishes to Bitcoin.
    fn start_writer(
        nodectx: &NodeContext,
        broadcast_handle: Arc<L1BroadcastHandle>,
    ) -> Result<(Arc<EnvelopeHandle>, DumbTickHandle)> {
        let sequencer_address = nodectx
            .task_manager()
            .handle()
            .block_on(generate_sequencer_address(nodectx.bitcoin_client()))?;

        let writer_db = nodectx.storage().db().writer_db();
        let config = Arc::new(nodectx.config().btcio.writer.clone());
        let btcio_params = super::build_btcio_params(
            nodectx.asm_params(),
            nodectx.config().btcio.l1_reorg_safe_depth,
        );
        let executor = nodectx.executor();

        nodectx.task_manager().handle().block_on(async {
            let writer_ops = Arc::new(EnvelopeDataOps::new(
                nodectx.storage().handle().clone(),
                writer_db,
            ));
            let (intent_tx, intent_rx) = mpsc::channel(64);
            let envelope_handle = Arc::new(EnvelopeHandle::new(writer_ops.clone(), intent_tx));

            let ctx = WriterContext::new(
                btcio_params,
                config.clone(),
                sequencer_address,
                nodectx.bitcoin_client().clone(),
                nodectx.status_channel().as_ref().clone(),
                Arc::new(AsmCheckpointInspector),
                Arc::new(CheckpointFailureCleanup::new(nodectx.storage().clone())),
            )
            .with_signing_mode_provider(Arc::new(CheckpointSequencerKeyProvider::new(
                nodectx.storage().clone(),
            )));
            let ctx = Arc::new(ctx);

            let (watcher_handle, _) = WatcherBuilder::new(
                ctx,
                writer_ops.clone(),
                broadcast_handle,
                Duration::from_millis(config.write_poll_dur_ms),
            )
            .launch(executor)
            .await?;

            let _ = BundlerBuilder::new(
                writer_ops,
                Duration::from_millis(config.bundle_interval_ms),
                intent_rx,
            )
            .launch(executor)
            .await?;

            Ok((envelope_handle, watcher_handle))
        })
    }

    /// Starts the OL block assembly service.
    ///
    /// Assembles OL blocks from mempool transactions.
    fn start_block_assembly(
        nodectx: &NodeContext,
        mempool_handle: Arc<MempoolHandle>,
    ) -> Result<BlockasmHandle> {
        let blockasm_config = nodectx
            .blockasm_config()
            .cloned()
            .ok_or_else(|| anyhow!("Block assembly config required for block assembly"))?;
        let sequencer_config = nodectx
            .config()
            .sequencer
            .clone()
            .ok_or_else(|| anyhow!("Sequencer config required for block assembly"))?;
        let sequencer_predicate = nodectx
            .asm_params()
            .checkpoint_config()
            .ok_or_else(|| anyhow!("ASM checkpoint config required for block assembly"))?
            .sequencer_predicate
            .clone();

        let epoch_sealing_config = nodectx.config().epoch_sealing.clone().unwrap_or_default();
        let slots_per_epoch = match epoch_sealing_config {
            EpochSealingConfig::FixedSlot { slots_per_epoch } => slots_per_epoch,
        };

        let mempool_provider = MempoolProviderImpl::new(mempool_handle);
        let epoch_sealing = LimitAwareSealing::new(FixedSlotSealing::new(slots_per_epoch));
        let state_provider = OLStateManagerProviderImpl::new(nodectx.storage().ol_state().clone());

        let l1_reorg_safe_depth = nodectx.config().btcio.l1_reorg_safe_depth;

        nodectx.task_manager().handle().block_on(async {
            BlockasmBuilder::new(
                nodectx.ol_params().clone(),
                blockasm_config,
                nodectx.storage().clone(),
                mempool_provider,
                epoch_sealing,
                state_provider,
                sequencer_config,
                sequencer_predicate,
                l1_reorg_safe_depth,
            )
            .launch(nodectx.executor())
            .await
        })
    }
}

#[cfg(not(feature = "sequencer"))]
mod sequencer_services {
    use std::sync::Arc;

    use anyhow::Result;
    use strata_node_context::NodeContext;
    use strata_ol_mempool::MempoolHandle;

    use crate::run_context::ServiceHandlesBuilder;

    pub(super) fn start_if_enabled(_: &NodeContext, _: Option<Arc<MempoolHandle>>) -> Result<()> {
        Ok(())
    }

    pub(super) fn attach_service_handles(
        builder: ServiceHandlesBuilder,
        _: (),
    ) -> ServiceHandlesBuilder {
        builder
    }
}

/// Proof notifier shared between the proof storer and the checkpoint worker.
pub(crate) type OptionalProofNotify = Option<Arc<strata_ol_checkpoint::ProofNotify>>;

#[cfg(feature = "sequencer")]
const CHECKPOINT_RECONCILE_POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(feature = "sequencer")]
const CHECKPOINT_RECONCILE_CATCHUP_TIMEOUT: Duration = Duration::from_secs(300);

/// Runs `reconcile` only after CSM catches up to ASM and retries if either tip
/// advances during the pass.
///
/// The catch-up wait is an optimization, not a precondition: reconciling against a
/// settled tip is what lets it cancel every stale artifact in one pass. A tip that
/// keeps moving is the normal shape of a restart with an L1 backlog, since the reader
/// starts before this runs and ASM ingests continuously while it drains. Giving up on
/// the wait therefore degrades to a single best-effort pass instead of failing, because
/// the caller aborts the process on `Err` and the backlog only grows across a restart,
/// so a hard failure here would crash-loop the sequencer exactly when it is furthest
/// behind. What the degraded pass still guarantees is the ordering that matters: it runs
/// before the broadcaster can republish anything, and the writer's stale-checkpoint gate
/// covers whatever a moving tip made it miss.
#[cfg(feature = "sequencer")]
fn reconcile_at_caught_up_checkpoint_tip(
    nodectx: &NodeContext,
    asm_handle: &AsmWorkerHandle,
    csm_monitor: &ServiceMonitor<CsmWorkerStatus>,
    reconcile: impl Fn(&NodeContext) -> Result<()>,
) -> Result<()> {
    let runtime = nodectx.task_manager().handle().clone();
    let deadline = Instant::now() + CHECKPOINT_RECONCILE_CATCHUP_TIMEOUT;

    loop {
        // Only the waiting runs on the runtime. `reconcile` blocks on storage, which
        // panics if it is driven from inside a runtime context, so it runs on this
        // thread between `block_on` calls, exactly as the non-sequencer path calls it.
        // The timer is also constructed inside the async block rather than passed as an
        // argument to `block_on`, because building a `Sleep` needs the current runtime.
        let remaining = deadline.saturating_duration_since(Instant::now());
        let caught_up = runtime.block_on(async {
            timeout(remaining, async {
                loop {
                    let asm_block = asm_handle.monitor().get_current().cur_block;
                    let csm_block = csm_monitor.get_current().cur_block;
                    if let Some(asm_block) = asm_block
                        && Some(asm_block) == csm_block
                    {
                        break asm_block;
                    }
                    sleep(CHECKPOINT_RECONCILE_POLL_INTERVAL).await;
                }
            })
            .await
        });

        let Ok(reconciled_block) = caught_up else {
            warn_catchup_timeout(asm_handle, csm_monitor, "reconciling against a moving tip");
            return reconcile(nodectx);
        };

        reconcile(nodectx)?;

        let asm_block = asm_handle.monitor().get_current().cur_block;
        let csm_block = csm_monitor.get_current().cur_block;
        if asm_block == Some(reconciled_block) && csm_block == Some(reconciled_block) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            warn_catchup_timeout(
                asm_handle,
                csm_monitor,
                "keeping the last pass, which ran against a tip that then moved",
            );
            return Ok(());
        }
    }
}

/// Reports that CSM never settled at the ASM tip in time, and how that is being absorbed.
#[cfg(feature = "sequencer")]
fn warn_catchup_timeout(
    asm_handle: &AsmWorkerHandle,
    csm_monitor: &ServiceMonitor<CsmWorkerStatus>,
    resolution: &str,
) {
    let asm_block = asm_handle.monitor().get_current().cur_block;
    let csm_block = csm_monitor.get_current().cur_block;
    warn!(
        ?asm_block,
        ?csm_block,
        resolution,
        "timed out after {CHECKPOINT_RECONCILE_CATCHUP_TIMEOUT:?} waiting for CSM to catch up \
         before checkpoint reconciliation"
    );
}

/// Starts services and returns the run context and an optional proof notifier.
///
/// The proof notifier is created when an integrated prover is configured. The
/// caller passes it to `start_prover_service` so that the proof storer can
/// wake the checkpoint worker immediately after storing a proof.
pub(crate) fn start_strata_services(
    nodectx: NodeContext,
) -> Result<(RunContext, OptionalProofNotify)> {
    // Start Asm worker
    let asm_handle = Arc::new(spawn_asm_worker_with_ctx(&nodectx)?);

    // Start Csm worker
    let csm_monitor = Arc::new(spawn_csm_listener_with_ctx(&nodectx, asm_handle.monitor())?);

    // btcio reader task must start before genesis init because genesis requires ASM to
    // have the genesis manifest which will be available only after btcio reader provides
    // the L1 block to ASM.
    start_btcio_reader(&nodectx, asm_handle.clone());

    // Check and do genesis if not yet. This should be done after asm/csm/btcio and before mempool
    // because genesis requires asm to be working and mempool and other services expect genesis to
    // have happened.
    ensure_genesis(
        nodectx.storage().as_ref(),
        nodectx.ol_params(),
        nodectx.status_channel().as_ref(),
    )?;

    let is_sequencer = nodectx.config().client.is_sequencer;
    if is_sequencer {
        #[cfg(feature = "sequencer")]
        reconcile_at_caught_up_checkpoint_tip(
            &nodectx,
            &asm_handle,
            &csm_monitor,
            reconcile_unaccepted_checkpoint_artifacts,
        )?;
    } else {
        reconcile_unaccepted_checkpoint_artifacts(&nodectx)?;
    }

    // Checkpoint sync nodes do not have mempool, so start mempool for sequencer node only.
    // NOTE: When there are nodes supporting mempool the if condition needs to change.
    let mempool_handle = if is_sequencer {
        Some(Arc::new(start_mempool(&nodectx)?))
    } else {
        None
    };

    // Start Chain worker
    let chain_worker_handle = Arc::new(start_chain_worker_service_from_ctx(&nodectx)?);

    // Start OL checkpoint service.
    // When an integrated prover is configured, the prover writes proofs to
    // the proof DB and signals ProofNotify to wake the checkpoint worker.
    // The worker waits indefinitely for proofs. Without a prover, empty
    // proofs are used immediately.
    let (checkpoint_handle, proof_notify) = if is_sequencer {
        let epoch_summary_rx = chain_worker_handle.subscribe_epoch_summaries();
        let checkpoint_builder = OLCheckpointBuilder::new()
            .with_node_context(&nodectx)
            .with_epoch_summary_receiver(epoch_summary_rx);

        #[cfg(feature = "prover")]
        let (checkpoint_builder, proof_notify): (
            OLCheckpointBuilder,
            Option<Arc<strata_ol_checkpoint::ProofNotify>>,
        ) = if nodectx.config().prover.is_some() {
            let notify = Arc::new(strata_ol_checkpoint::ProofNotify::new());
            let builder = checkpoint_builder.with_prover(strata_ol_checkpoint::ProverConfig {
                notify: notify.clone(),
            });
            (builder, Some(notify))
        } else {
            (checkpoint_builder, None)
        };

        #[cfg(not(feature = "prover"))]
        let proof_notify: Option<Arc<strata_ol_checkpoint::ProofNotify>> = None;

        let handle = Arc::new(checkpoint_builder.launch(nodectx.executor())?);
        (Some(handle), proof_notify)
    } else {
        (None, None)
    };

    if is_sequencer {
        #[cfg(feature = "sequencer")]
        reconcile_at_caught_up_checkpoint_tip(
            &nodectx,
            &asm_handle,
            &csm_monitor,
            reconcile_settled_checkpoint_queue,
        )?;
    }
    let sequencer_handles = sequencer_services::start_if_enabled(&nodectx, mempool_handle.clone())?;

    let sync_handle =
        start_sync_services(&nodectx, chain_worker_handle.clone(), csm_monitor.clone())?;

    let service_handles_builder = ServiceHandles::builder(
        asm_handle,
        csm_monitor,
        mempool_handle,
        chain_worker_handle,
        checkpoint_handle,
        sync_handle,
    );
    let service_handles =
        sequencer_services::attach_service_handles(service_handles_builder, sequencer_handles)
            .build();

    let runctx = RunContext::from_node_ctx(nodectx, service_handles);

    #[cfg(feature = "prover")]
    return Ok((runctx, proof_notify));

    #[cfg(not(feature = "prover"))]
    Ok((runctx, proof_notify))
}

/// Starts the btcio reader task.
///
/// Polls Bitcoin for new blocks and submits them to ASM for processing.
fn start_btcio_reader(nodectx: &NodeContext, asm_handle: Arc<strata_asm_worker::AsmWorkerHandle>) {
    nodectx.executor().spawn_critical_async(
        "bitcoin_data_reader_task",
        bitcoin_data_reader_task(
            nodectx.bitcoin_client().clone(),
            nodectx.storage().clone(),
            Arc::new(nodectx.config().btcio.reader.clone()),
            build_btcio_params(
                nodectx.asm_params(),
                nodectx.config().btcio.l1_reorg_safe_depth,
            ),
            ReaderValidation::new(
                nodectx.config().bitcoind.network,
                nodectx.ol_params().last_l1_block,
            ),
            nodectx.status_channel().as_ref().clone(),
            Arc::new(AsmBlockSubmitter::new(asm_handle)),
        ),
    );
}

/// Starts the OL sync service for the node's role.
///
/// Sequencer nodes run the fork-choice manager; non-sequencer nodes run the
/// checkpoint sync service. A node runs exactly one.
fn start_sync_services(
    nodectx: &NodeContext,
    chain_worker_handle: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
) -> Result<SyncServiceHandle> {
    if nodectx.config().client.is_sequencer {
        let fcm_handle = fcm::start(nodectx, chain_worker_handle, csm_monitor)?;
        Ok(SyncServiceHandle::Fcm(Arc::new(fcm_handle)))
    } else {
        let css_handle = css::start(nodectx, chain_worker_handle, csm_monitor)?;
        Ok(SyncServiceHandle::Css(Arc::new(css_handle)))
    }
}

/// Starts the mempool service.
fn start_mempool(nodectx: &NodeContext) -> Result<MempoolHandle> {
    let config = OLMempoolConfig::default();

    let current_tip = nodectx
        .status_channel()
        .get_ol_sync_status()
        .expect("OL sync status must be set before starting mempool")
        .tip();

    let storage = nodectx.storage().clone();
    let status_channel = nodectx.status_channel().as_ref().clone();
    let executor = nodectx.executor().clone();

    // block_on is required because start_services is synchronous but we need
    // to initialize the mempool which requires async operations. The mempool
    // handle must be available before RunContext is constructed.
    nodectx.task_manager().handle().block_on(async {
        MempoolBuilder::new(config, storage, status_channel, current_tip)
            .launch(&executor)
            .await
    })
}
