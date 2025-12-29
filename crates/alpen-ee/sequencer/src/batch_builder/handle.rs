//! Builder and handle for the batch builder task.

use std::{future::Future, sync::Arc};

use alpen_ee_common::{BatchId, BatchStorage, ExecBlockStorage};
use alpen_ee_exec_chain::ExecChainHandle;
use strata_acct_types::Hash;
use tokio::sync::watch;

use super::{
    ctx::BatchBuilderCtx, task::batch_builder_task, BatchBuilderConfig, BatchBuilderState,
    BatchPolicy, BatchSealingPolicy, BlockDataProvider,
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
    latest_batch_rx: watch::Receiver<Option<BatchId>>,
}

impl BatchBuilderHandle {
    /// Returns a receiver that can be used to watch for batch updates.
    pub fn latest_batch_watcher(&self) -> watch::Receiver<Option<BatchId>> {
        self.latest_batch_rx.clone()
    }

    /// Returns the current latest batch ID, if any.
    pub fn latest_batch_id(&self) -> Option<BatchId> {
        *self.latest_batch_rx.borrow()
    }
}

/// Default backoff duration (ms) when block data is not yet available.
const DEFAULT_DATA_POLL_INTERVAL_MS: u64 = 100;
/// Default backoff duration (ms) on errors.
const DEFAULT_ERROR_BACKOFF_MS: u64 = 1000;
/// Default maximum blocks per batch.
const DEFAULT_MAX_BLOCKS_PER_BATCH: u64 = 100;

/// Builder for creating a batch builder task with custom configuration.
#[derive(Debug)]
pub struct BatchBuilderBuilder<P, D, S, BS, ES>
where
    P: BatchPolicy,
{
    genesis_hash: Hash,
    state: BatchBuilderState<P>,
    preconf_rx: watch::Receiver<Hash>,
    block_data_provider: Arc<D>,
    sealing_policy: S,
    block_storage: Arc<ES>,
    batch_storage: Arc<BS>,
    exec_chain: Arc<ExecChainHandle>,
    max_blocks_per_batch: Option<u64>,
    data_poll_interval_ms: Option<u64>,
    error_backoff_ms: Option<u64>,
}

impl<P, D, S, BS, ES> BatchBuilderBuilder<P, D, S, BS, ES>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    #[expect(clippy::too_many_arguments, reason = "required builder fields")]
    /// Creates a new batch builder builder with all required fields.
    pub fn new(
        genesis_hash: Hash,
        state: BatchBuilderState<P>,
        preconf_rx: watch::Receiver<Hash>,
        block_data_provider: Arc<D>,
        sealing_policy: S,
        block_storage: Arc<ES>,
        batch_storage: Arc<BS>,
        exec_chain: Arc<ExecChainHandle>,
    ) -> Self {
        Self {
            genesis_hash,
            state,
            preconf_rx,
            block_data_provider,
            sealing_policy,
            block_storage,
            batch_storage,
            exec_chain,
            max_blocks_per_batch: None,
            data_poll_interval_ms: None,
            error_backoff_ms: None,
        }
    }

    /// Sets the maximum number of blocks per batch.
    pub fn with_max_blocks_per_batch(mut self, v: u64) -> Self {
        self.max_blocks_per_batch = Some(v);
        self
    }

    /// Sets the polling interval (ms) when waiting for block data.
    pub fn with_data_poll_interval_ms(mut self, v: u64) -> Self {
        self.data_poll_interval_ms = Some(v);
        self
    }

    /// Sets the error backoff duration in milliseconds.
    pub fn with_error_backoff_ms(mut self, v: u64) -> Self {
        self.error_backoff_ms = Some(v);
        self
    }

    /// Builds and returns the batch builder handle and task.
    ///
    /// The handle provides a watch channel for observing the latest batch ID.
    pub fn build(
        self,
        initial_batch_id: Option<BatchId>,
    ) -> (BatchBuilderHandle, impl Future<Output = ()>) {
        let config = BatchBuilderConfig {
            max_blocks_per_batch: self
                .max_blocks_per_batch
                .unwrap_or(DEFAULT_MAX_BLOCKS_PER_BATCH),
            data_poll_interval_ms: self
                .data_poll_interval_ms
                .unwrap_or(DEFAULT_DATA_POLL_INTERVAL_MS),
            error_backoff_ms: self.error_backoff_ms.unwrap_or(DEFAULT_ERROR_BACKOFF_MS),
        };

        let (latest_batch_tx, latest_batch_rx) = watch::channel(initial_batch_id);

        let ctx = BatchBuilderCtx {
            genesis_hash: self.genesis_hash,
            config,
            preconf_rx: self.preconf_rx,
            block_data_provider: self.block_data_provider,
            sealing_policy: self.sealing_policy,
            block_storage: self.block_storage,
            batch_storage: self.batch_storage,
            exec_chain: self.exec_chain,
            latest_batch_tx,
            _policy: std::marker::PhantomData,
        };

        let handle = BatchBuilderHandle { latest_batch_rx };
        let task = batch_builder_task(self.state, ctx);

        (handle, task)
    }
}
