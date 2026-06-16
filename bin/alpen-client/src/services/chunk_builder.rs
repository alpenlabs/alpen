use std::{sync::Arc, time::Duration};

use alpen_ee_common::{BatchStorage, BlockNumHash, ChunkStorage, ExecBlockStorage};
use alpen_ee_sequencer::{
    chunk_builder::{
        cleanup_orphaned_chunks, create_chunk_builder_state, init_chunk_builder_state,
        repair_batch_linkage, ChunkBuilderService, ChunkBuilderStatus,
    },
    sealing_policy::{
        block_count_policy::{BlockCountDataProvider, BlockCountPolicy, FixedBlockCountSealing},
        gas_limit_policy::{GasLimitPolicy, MaxGasSealing},
        or_policy::{ComposedDataProvider, ComposedPolicy, OrSealing},
        BlockDataProvider,
    },
    BatchBuilderEvent, ChunkExtractRequest,
};
use strata_service::{AsyncExecutor, ServiceBuilder, ServiceMonitor, TickingInput, TokioMpscInput};
use tokio::sync::mpsc;

/// Polling interval for retrying pending blocks whose data isn't ready.
const PENDING_BLOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Concrete policy: block-count OR gas-limit, whichever triggers first.
type ChunkPolicy = ComposedPolicy<BlockCountPolicy, GasLimitPolicy>;
type ChunkSealing =
    OrSealing<BlockCountPolicy, GasLimitPolicy, FixedBlockCountSealing, MaxGasSealing>;

/// Starts the chunk builder as a service framework service.
///
/// Runs orphan cleanup synchronously (fast consistency fix), then
/// launches the service. Backfill of unchunked batches runs in
/// `on_launch` on the service's worker task.
#[expect(clippy::too_many_arguments, reason = "service constructor")]
pub(crate) async fn start_chunk_builder_service<CS, BS, ES, GD>(
    genesis: BlockNumHash,
    chunk_storage: Arc<CS>,
    batch_storage: Arc<BS>,
    block_storage: Arc<ES>,
    sealing_policy: ChunkSealing,
    gas_data_provider: GD,
    chunk_witness_tx: Option<mpsc::Sender<ChunkExtractRequest>>,
    event_rx: mpsc::Receiver<BatchBuilderEvent>,
    executor: &impl AsyncExecutor,
) -> anyhow::Result<ServiceMonitor<ChunkBuilderStatus>>
where
    CS: ChunkStorage + 'static,
    BS: BatchStorage + 'static,
    ES: ExecBlockStorage + 'static,
    GD: BlockDataProvider<GasLimitPolicy> + 'static,
{
    // Revert chunks past the last complete batch boundary.
    cleanup_orphaned_chunks(chunk_storage.as_ref(), batch_storage.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("chunk orphan cleanup: {e}"))?;

    // Repair batch-chunk linkage if missing (crash before boundary).
    repair_batch_linkage(chunk_storage.as_ref(), batch_storage.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("repair batch linkage: {e}"))?;

    // Load state from (now-consistent) storage.
    let state = init_chunk_builder_state::<ChunkPolicy>(chunk_storage.as_ref(), genesis)
        .await
        .map_err(|e| anyhow::anyhow!("init_chunk_builder_state: {e}"))?;

    let block_data_provider = Arc::new(ComposedDataProvider::new(
        BlockCountDataProvider,
        gas_data_provider,
    ));

    let svc_state = create_chunk_builder_state(
        state,
        sealing_policy,
        block_data_provider,
        chunk_witness_tx,
        chunk_storage,
        batch_storage,
        block_storage,
    );

    let input = TickingInput::new(PENDING_BLOCK_POLL_INTERVAL, TokioMpscInput::new(event_rx));

    ServiceBuilder::<
        ChunkBuilderService<ChunkPolicy, ChunkSealing, _, CS, BS, ES>,
        TickingInput<TokioMpscInput<BatchBuilderEvent>>,
    >::new()
    .with_state(svc_state)
    .with_input(input)
    .launch_async("ee_chunk_builder", executor)
    .await
}
