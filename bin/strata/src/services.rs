//! Service spawning and lifecycle management.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use strata_btcio::reader::query::bitcoin_data_reader_task;
use strata_chain_worker_new::start_chain_worker_service_from_ctx;
use strata_consensus_logic::{
    FcmContext, start_fcm_service,
    sync_manager::{spawn_asm_worker_with_ctx, spawn_csm_listener_with_ctx},
};
use strata_identifiers::OLBlockCommitment;
use strata_node_context::NodeContext;
use strata_ol_checkpoint::OLCheckpointBuilder;
use strata_ol_mempool::{MempoolBuilder, MempoolHandle, OLMempoolConfig};

use crate::{
    context::check_and_init_genesis,
    helpers::rollup_to_btcio_params,
    run_context::{RunContext, ServiceHandles},
};

#[cfg(feature = "sequencer")]
mod sequencer_services {
    use std::{sync::Arc, time::Duration};

    use anyhow::{Result, anyhow};
    use strata_btcio::{
        broadcaster::{L1BroadcastHandle, spawn_broadcaster_task},
        writer::{EnvelopeHandle, start_envelope_task},
    };
    use strata_config::EpochSealingConfig;
    use strata_db_types::traits::DatabaseBackend;
    use strata_node_context::NodeContext;
    use strata_ol_block_assembly::{
        BlockasmBuilder, BlockasmHandle, FixedSlotSealing, MempoolProviderImpl,
    };
    use strata_ol_mempool::MempoolHandle;
    use strata_ol_sequencer::TemplateManager;
    use strata_storage::ops::l1tx_broadcast;

    use crate::{
        helpers::generate_sequencer_address,
        run_context::{SequencerServiceHandles, ServiceHandlesBuilder},
    };

    /// TTL for template manager.
    // TODO: make this configurable.
    const TEMPLATE_MANAGER_TTL: Duration = Duration::from_secs(60);

    pub(super) fn start_if_enabled(
        nodectx: &NodeContext,
        mempool_handle: Arc<MempoolHandle>,
    ) -> Result<Option<SequencerServiceHandles>> {
        if !nodectx.config().client.is_sequencer {
            return Ok(None);
        }

        let broadcast_handle = Arc::new(start_broadcaster(nodectx));
        let envelope_handle = start_writer(nodectx, broadcast_handle.clone())?;
        let blockasm_handle = Arc::new(start_block_assembly(nodectx, mempool_handle)?);
        let template_manager = Arc::new(TemplateManager::new(
            blockasm_handle,
            nodectx.storage().clone(),
            TEMPLATE_MANAGER_TTL,
        ));

        Ok(Some(SequencerServiceHandles::new(
            broadcast_handle,
            envelope_handle,
            template_manager,
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
    fn start_broadcaster(nodectx: &NodeContext) -> L1BroadcastHandle {
        let broadcast_db = nodectx.storage().db().broadcast_db();
        let broadcast_ctx = l1tx_broadcast::Context::new(broadcast_db);
        let broadcast_ops = Arc::new(broadcast_ctx.into_ops(nodectx.storage().pool().clone()));

        spawn_broadcaster_task(
            nodectx.executor(),
            nodectx.bitcoin_client().clone(),
            broadcast_ops,
            super::rollup_to_btcio_params(nodectx.params().rollup()),
            nodectx.config().btcio.broadcaster.poll_interval_ms,
        )
    }

    /// Starts the L1 writer/envelope task.
    ///
    /// Bundles L1 intents, creates envelope transactions, and publishes to Bitcoin.
    fn start_writer(
        nodectx: &NodeContext,
        broadcast_handle: Arc<L1BroadcastHandle>,
    ) -> Result<Arc<EnvelopeHandle>> {
        let sequencer_address = nodectx
            .task_manager()
            .handle()
            .block_on(generate_sequencer_address(nodectx.bitcoin_client()))?;

        let writer_db = nodectx.storage().db().writer_db();

        start_envelope_task(
            nodectx.executor(),
            nodectx.bitcoin_client().clone(),
            Arc::new(nodectx.config().btcio.writer.clone()),
            super::rollup_to_btcio_params(nodectx.params().rollup()),
            sequencer_address,
            writer_db,
            nodectx.status_channel().as_ref().clone(),
            nodectx.storage().pool().clone(),
            broadcast_handle,
        )
    }

    /// Starts the OL block assembly service.
    ///
    /// Assembles OL blocks from mempool transactions.
    fn start_block_assembly(
        nodectx: &NodeContext,
        mempool_handle: Arc<MempoolHandle>,
    ) -> Result<BlockasmHandle> {
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
        let state_provider = nodectx.storage().ol_state().clone();

        nodectx.task_manager().handle().block_on(async {
            BlockasmBuilder::new(
                nodectx.params().clone(),
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

    pub(super) fn start_if_enabled(_: &NodeContext, _: Arc<MempoolHandle>) -> Result<()> {
        Ok(())
    }

    pub(super) fn attach_service_handles(
        builder: ServiceHandlesBuilder,
        _: (),
    ) -> ServiceHandlesBuilder {
        builder
    }
}

/// Just simply starts services. This can later be extended to service registry pattern.
pub(crate) fn start_strata_services(nodectx: NodeContext) -> Result<RunContext> {
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
    check_and_init_genesis(
        nodectx.storage().as_ref(),
        nodectx.params().as_ref(),
        nodectx.status_channel().as_ref(),
    )?;

    // Start mempool service
    let mempool_handle = Arc::new(start_mempool(&nodectx)?);

    // Start Chain worker
    let chain_worker_handle = Arc::new(start_chain_worker_service_from_ctx(&nodectx)?);

    // Start OL checkpoint service
    let epoch_summary_rx = chain_worker_handle.subscribe_epoch_summaries();
    let checkpoint_handle = Arc::new(
        OLCheckpointBuilder::new()
            .with_node_context(&nodectx)
            .with_epoch_summary_receiver(epoch_summary_rx)
            .launch(nodectx.executor())?,
    );

    let sequencer_handles = sequencer_services::start_if_enabled(&nodectx, mempool_handle.clone())?;

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

    Ok(RunContext::from_node_ctx(nodectx, service_handles))
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
            asm_handle,
        ),
    );
}

/// Starts the mempool service.
fn start_mempool(nodectx: &NodeContext) -> Result<MempoolHandle> {
    let config = OLMempoolConfig::default();

    // Get current chain tip - try status channel first, fall back to genesis from storage
    let current_tip = match nodectx.status_channel().get_ol_sync_status() {
        Some(status) => status.tip,
        None => {
            // No chain sync status yet - get genesis block from OL storage
            let genesis_blocks = nodectx
                .storage()
                .ol_block()
                .get_blocks_at_height_blocking(0)
                .map_err(|e| anyhow!("Failed to get genesis block: {e}"))?;
            let genesis_blkid = genesis_blocks
                .first()
                .ok_or_else(|| anyhow!("Genesis block not found, cannot start mempool"))?;
            OLBlockCommitment::new(0, *genesis_blkid)
        }
    };

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
