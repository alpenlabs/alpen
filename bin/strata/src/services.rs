//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::Result;
use strata_btcio::reader::query::bitcoin_data_reader_task;
use strata_chain_worker_new::start_chain_worker_service_from_ctx;
use strata_consensus_logic::{
    AsmBlockSubmitter, FcmContext, start_fcm_service,
    sync_manager::{spawn_asm_worker_with_ctx, spawn_csm_listener_with_ctx},
};
use strata_node_context::NodeContext;
use strata_ol_checkpoint::OLCheckpointBuilder;
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};

use crate::{
    context::ensure_genesis,
    helpers::rollup_to_btcio_params,
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
    use strata_db_types::traits::DatabaseBackend;
    use strata_node_context::NodeContext;
    use strata_ol_block_assembly::{
        BlockasmBuilder, BlockasmHandle, FixedSlotSealing, MempoolProviderImpl,
    };
    use strata_ol_mempool::MempoolHandle;
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_service::DumbTickHandle;
    use strata_storage::ops::{l1tx_broadcast, writer::Context};
    use tokio::sync::mpsc;

    use crate::{
        helpers::generate_sequencer_address,
        run_context::{SequencerServiceHandles, ServiceHandlesBuilder},
    };

    pub(super) fn start_if_enabled(
        nodectx: &NodeContext,
        mempool_handle: Arc<MempoolHandle>,
        envelope_pubkey: Option<[u8; 32]>,
    ) -> Result<Option<SequencerServiceHandles>> {
        if !nodectx.config().client.is_sequencer {
            return Ok(None);
        }

        let broadcast_handle = Arc::new(start_broadcaster(nodectx)?);
        let (envelope_handle, watcher_handle) =
            start_writer(nodectx, broadcast_handle.clone(), envelope_pubkey)?;
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
        let broadcast_ctx = l1tx_broadcast::Context::new(broadcast_db);
        let broadcast_ops = Arc::new(broadcast_ctx.into_ops(nodectx.storage().pool().clone()));

        nodectx.task_manager().handle().block_on(async {
            BroadcasterBuilder::new(
                nodectx.bitcoin_client().clone(),
                broadcast_ops,
                super::rollup_to_btcio_params(nodectx.params().rollup()),
            )
            .with_broadcast_poll_interval_ms(nodectx.config().btcio.broadcaster.poll_interval_ms)
            .launch(nodectx.executor())
            .await
        })
    }

    /// Starts the L1 writer/envelope task.
    ///
    /// Bundles L1 intents, creates envelope transactions, and publishes to Bitcoin.
    fn start_writer(
        nodectx: &NodeContext,
        broadcast_handle: Arc<L1BroadcastHandle>,
        envelope_pubkey: Option<[u8; 32]>,
    ) -> Result<(Arc<EnvelopeHandle>, DumbTickHandle)> {
        let sequencer_address = nodectx
            .task_manager()
            .handle()
            .block_on(generate_sequencer_address(nodectx.bitcoin_client()))?;

        let writer_db = nodectx.storage().db().writer_db();
        let config = Arc::new(nodectx.config().btcio.writer.clone());
        let btcio_params = super::rollup_to_btcio_params(nodectx.params().rollup());
        let executor = nodectx.executor();

        nodectx.task_manager().handle().block_on(async {
            let writer_ops =
                Arc::new(Context::new(writer_db).into_ops(nodectx.storage().pool().clone()));
            let (intent_tx, intent_rx) = mpsc::channel(64);
            let envelope_handle = Arc::new(EnvelopeHandle::new(writer_ops.clone(), intent_tx));

            let mut ctx = WriterContext::new(
                btcio_params,
                config.clone(),
                sequencer_address,
                nodectx.bitcoin_client().clone(),
                nodectx.status_channel().as_ref().clone(),
            );
            if let Some(pk) = &envelope_pubkey {
                ctx = ctx.with_envelope_pubkey(pk);
            }
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

        let epoch_sealing_config = nodectx.config().epoch_sealing.clone().unwrap_or_default();
        let slots_per_epoch = match epoch_sealing_config {
            EpochSealingConfig::FixedSlot { slots_per_epoch } => slots_per_epoch,
        };

        let mempool_provider = MempoolProviderImpl::new(mempool_handle);
        let epoch_sealing = FixedSlotSealing::new(slots_per_epoch);
        let state_provider = OLStateManagerProviderImpl::new(nodectx.storage().ol_state().clone());

        nodectx.task_manager().handle().block_on(async {
            BlockasmBuilder::new(
                nodectx.params().clone(),
                blockasm_config,
                nodectx.storage().clone(),
                mempool_provider,
                epoch_sealing,
                state_provider,
                sequencer_config,
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

    pub(super) fn start_if_enabled(
        _: &NodeContext,
        _: Arc<MempoolHandle>,
        _: Option<[u8; 32]>,
    ) -> Result<()> {
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

/// Starts services and returns the run context and an optional proof notifier.
///
/// The proof notifier is created when an integrated prover is configured. The
/// caller passes it to `start_prover_service` so that the proof storer can
/// wake the checkpoint worker immediately after storing a proof.
pub(crate) fn start_strata_services(
    nodectx: NodeContext,
    envelope_pubkey: Option<[u8; 32]>,
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

    // Start mempool service
    let mempool_handle = Arc::new(start_mempool(&nodectx)?);

    // Start Chain worker
    let chain_worker_handle = Arc::new(start_chain_worker_service_from_ctx(&nodectx)?);

    // Start OL checkpoint service.
    // When an integrated prover is configured, the prover writes proofs to
    // the proof DB and signals ProofNotify to wake the checkpoint worker.
    // The worker waits indefinitely for proofs. Without a prover, empty
    // proofs are used immediately.
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

    let checkpoint_handle = Arc::new(checkpoint_builder.launch(nodectx.executor())?);

    let sequencer_handles =
        sequencer_services::start_if_enabled(&nodectx, mempool_handle.clone(), envelope_pubkey)?;

    let fcm_ctx =
        FcmContext::from_node_ctx(&nodectx, chain_worker_handle.clone(), csm_monitor.clone());

    let fcm_handle = nodectx
        .task_manager()
        .handle()
        .block_on(start_fcm_service(fcm_ctx, nodectx.executor().clone()))?;
    let fcm_handle = Arc::new(fcm_handle);

    let service_handles_builder = ServiceHandles::builder(
        asm_handle,
        csm_monitor,
        mempool_handle,
        chain_worker_handle,
        checkpoint_handle,
        fcm_handle,
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
            rollup_to_btcio_params(nodectx.params().rollup()),
            nodectx.status_channel().as_ref().clone(),
            Arc::new(AsmBlockSubmitter::new(asm_handle)),
        ),
    );
}

/// Starts the mempool service.
fn start_mempool(nodectx: &NodeContext) -> Result<MempoolHandle> {
    let config = OLMempoolConfig::default();

    let current_tip = nodectx
        .status_channel()
        .get_ol_sync_status()
        .expect("OL sync status must be set before starting mempool")
        .tip;

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
