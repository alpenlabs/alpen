//! Builder and handle for the batch builder task.

use std::{future::Future, marker::PhantomData, sync::Arc};

use alpen_ee_common::{BatchId, BatchStorage, BlockNumHash, ExecBlockStorage};
use alpen_ee_exec_chain::ExecChainHandle;
use tokio::sync::watch;

use super::{
    ctx::BatchBuilderCtx, task::batch_builder_task, BatchBuilderState, BatchPolicy,
    BatchSealingPolicy, BlockDataProvider,
};

/// Handle to observe batch builder state changes.
///
/// Provides a watch channel that is updated whenever:
/// - A new batch is sealed
/// - A reorg causes batches to be reverted
#[derive(Debug, Clone)]
pub struct BatchBuilderHandle {
    /// Receiver for the latest batch ID.
    /// The value is `None` if no batches exist yet, otherwise `Some(latest_batch_id)`.
    latest_batch_rx: watch::Receiver<BatchId>,
}

impl BatchBuilderHandle {
    /// Returns a receiver that can be used to watch for batch updates.
    pub fn latest_batch_watcher(&self) -> watch::Receiver<BatchId> {
        self.latest_batch_rx.clone()
    }

    /// Returns the current latest batch ID.
    pub fn latest_batch_id(&self) -> BatchId {
        *self.latest_batch_rx.borrow()
    }
}

/// Create batch builder task.
#[expect(
    clippy::too_many_arguments,
    reason = "all args are required to create the task"
)]
pub fn create_batch_builder<P, D, S, BS, ES>(
    initial_batch_id: BatchId,
    genesis: BlockNumHash,
    state: BatchBuilderState<P>,
    preconf_rx: watch::Receiver<BlockNumHash>,
    block_data_provider: Arc<D>,
    sealing_policy: S,
    block_storage: Arc<ES>,
    batch_storage: Arc<BS>,
    exec_chain: ExecChainHandle,
) -> (BatchBuilderHandle, impl Future<Output = ()>)
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    let (latest_batch_tx, latest_batch_rx) = watch::channel(initial_batch_id);

    let ctx = BatchBuilderCtx {
        genesis,
        preconf_rx,
        block_data_provider,
        sealing_policy,
        block_storage,
        batch_storage,
        exec_chain,
        latest_batch_tx,
        _policy: PhantomData,
    };

    let handle = BatchBuilderHandle { latest_batch_rx };
    let task = batch_builder_task(state, ctx);

    (handle, task)
}
