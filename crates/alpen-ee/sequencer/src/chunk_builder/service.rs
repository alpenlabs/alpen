//! Service framework integration for the chunk builder.

use std::{fmt, marker::PhantomData, sync::Arc};

use alpen_ee_common::{BatchStorage, ChunkStorage, ExecBlockStorage};
use serde::Serialize;
use strata_service::{AsyncService, Response, Service, ServiceState, TickMsg};
use tokio::sync::mpsc;
use tracing::error;

use super::{
    handlers, recovery,
    state::{ChunkBuilderState, PendingEntry},
};
use crate::{
    policy::{AccumulationPolicy, BlockDataProvider, SealingPolicy},
    BatchBuilderEvent, ChunkExtractRequest,
};

/// Create the chunk builder service state from its components.
pub fn create_chunk_builder_state<P, S, D, CS, BS, ES>(
    state: ChunkBuilderState<P>,
    sealing_policy: S,
    block_data_provider: Arc<D>,
    chunk_witness_tx: Option<mpsc::Sender<ChunkExtractRequest>>,
    chunk_storage: Arc<CS>,
    batch_storage: Arc<BS>,
    block_storage: Arc<ES>,
) -> ChunkBuilderServiceState<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
    S: SealingPolicy<P>,
    D: BlockDataProvider<P>,
    CS: ChunkStorage,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    ChunkBuilderServiceState {
        chunk_state: state,
        chunk_storage,
        sealing_policy,
        block_data_provider,
        chunk_witness_tx,
        batch_storage,
        block_storage,
    }
}

/// Chunk builder service marker type.
#[derive(Debug)]
pub struct ChunkBuilderService<P, S, D, CS, BS, ES>(PhantomData<(P, S, D, CS, BS, ES)>);

/// Minimal status for the service framework.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ChunkBuilderStatus;

/// Service state for the chunk builder.
pub struct ChunkBuilderServiceState<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
{
    pub(crate) chunk_state: ChunkBuilderState<P>,
    pub(crate) chunk_storage: Arc<CS>,
    pub(crate) sealing_policy: S,
    pub(crate) block_data_provider: Arc<D>,
    pub(crate) chunk_witness_tx: Option<mpsc::Sender<ChunkExtractRequest>>,
    pub(crate) batch_storage: Arc<BS>,
    pub(crate) block_storage: Arc<ES>,
}

impl<P, S, D, CS, BS, ES> fmt::Debug for ChunkBuilderServiceState<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkBuilderServiceState")
            .finish_non_exhaustive()
    }
}

impl<P, S, D, CS, BS, ES> ServiceState for ChunkBuilderServiceState<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
    S: Send + Sync + 'static,
    D: Send + Sync + 'static,
    CS: Send + Sync + 'static,
    BS: Send + Sync + 'static,
    ES: Send + Sync + 'static,
{
    fn name(&self) -> &str {
        "chunk_builder"
    }

    fn span_prefix(&self) -> &str {
        "chunk_builder"
    }
}

impl<P, S, D, CS, BS, ES> Service for ChunkBuilderService<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
    S: SealingPolicy<P> + 'static,
    D: BlockDataProvider<P> + 'static,
    CS: ChunkStorage + 'static,
    BS: BatchStorage + 'static,
    ES: ExecBlockStorage + 'static,
{
    type State = ChunkBuilderServiceState<P, S, D, CS, BS, ES>;
    type Msg = TickMsg<BatchBuilderEvent>;
    type Status = ChunkBuilderStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        ChunkBuilderStatus
    }
}

impl<P, S, D, CS, BS, ES> AsyncService for ChunkBuilderService<P, S, D, CS, BS, ES>
where
    P: AccumulationPolicy,
    S: SealingPolicy<P> + 'static,
    D: BlockDataProvider<P> + 'static,
    CS: ChunkStorage + 'static,
    BS: BatchStorage + 'static,
    ES: ExecBlockStorage + 'static,
{
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        recovery::enqueue_backfill(
            &mut state.chunk_state,
            state.batch_storage.as_ref(),
            state.block_storage.as_ref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("chunk builder backfill: {e}"))
    }

    async fn process_input(
        state: &mut Self::State,
        input: TickMsg<BatchBuilderEvent>,
    ) -> anyhow::Result<Response> {
        let result = match input {
            TickMsg::Msg(BatchBuilderEvent::BlockProcessed {
                block,
                batch_idx,
                batch_sealed,
            }) => {
                if let Some(batch_id) = batch_sealed {
                    state
                        .chunk_state
                        .push_pending(PendingEntry::BatchBoundary(batch_id));
                }
                state
                    .chunk_state
                    .push_pending(PendingEntry::Block { block, batch_idx });
                Ok(())
            }
            TickMsg::Msg(BatchBuilderEvent::Reorg {
                revert_to,
                last_valid_batch_idx,
            }) => {
                handlers::handle_reorg(
                    &mut state.chunk_state,
                    state.chunk_storage.as_ref(),
                    state.batch_storage.as_ref(),
                    state.block_storage.as_ref(),
                    revert_to,
                    last_valid_batch_idx,
                )
                .await
            }
            TickMsg::Tick => {
                handlers::process_pending(
                    &mut state.chunk_state,
                    state.chunk_storage.as_ref(),
                    &state.sealing_policy,
                    state.block_data_provider.as_ref(),
                    state.chunk_witness_tx.as_ref(),
                )
                .await
            }
        };

        if let Err(e) = &result {
            error!(error = %e, "chunk builder error");
        }

        result.map_err(|e| anyhow::anyhow!(e))?;
        Ok(Response::Continue)
    }
}
