//! Context for the batch builder task.

use std::{marker::PhantomData, sync::Arc};

use alpen_ee_common::{
    BatchId, BatchStorage, BlockNumHash, ChunkStorage, ChunkWitnessStore, ExecBlockStorage,
};
use alpen_ee_exec_chain::ExecChainHandle;
use tokio::sync::{mpsc, watch};

use crate::{
    batch_builder::canonical::{CanonicalChainReader, ExecChainCanonicalReader},
    chunk_witness_task::ChunkExtractRequest,
    policy::{AccumulationPolicy, BlockDataProvider, SealingPolicy},
};

/// Context holding all dependencies for the batch builder task.
///
/// This struct contains everything the task needs except for the mutable state,
/// which is passed separately to allow for state recovery on restart.
pub(crate) struct BatchBuilderCtx<P, D, S, BS, ES>
where
    P: AccumulationPolicy,
    D: BlockDataProvider<P>,
    S: SealingPolicy<P>,
    BS: BatchStorage + ChunkStorage + ChunkWitnessStore,
    ES: ExecBlockStorage,
{
    /// Genesis block hash, used as the starting point for the first batch.
    pub genesis: BlockNumHash,
    /// Receiver for canonical tip updates from ExecChain.
    pub preconf_rx: watch::Receiver<BlockNumHash>,
    /// Provider for fetching block data (e.g., DA size).
    pub block_data_provider: Arc<D>,
    /// Policy for determining when to seal a batch.
    pub sealing_policy: S,
    /// Storage for exec blocks.
    pub block_storage: Arc<ES>,
    /// Storage for batches + chunk witnesses (single concrete type
    /// implements both today, but the bounds keep the two concerns
    /// distinct).
    pub batch_storage: Arc<BS>,
    /// Handle to query canonical chain status.
    pub exec_chain: ExecChainHandle,
    /// Sender to notify about latest batch updates (new batch sealed or reorg).
    pub latest_batch_tx: watch::Sender<BatchId>,
    /// Optional channel to the background chunk-witness task. When
    /// present, `seal_batch` publishes a `ChunkExtractRequest` per
    /// sealed chunk and the witness is computed off the builder's hot
    /// path. When absent (tests, configurations without a reth
    /// provider), chunks seal with no witness pre-computed and the
    /// chunk prover will see a `TransientFailure` on the missing
    /// record.
    pub chunk_witness_tx: Option<mpsc::Sender<ChunkExtractRequest>>,
    /// Marker for the policy type.
    pub _policy: PhantomData<P>,
}

impl<P, D, S, BS, ES> BatchBuilderCtx<P, D, S, BS, ES>
where
    P: AccumulationPolicy,
    D: BlockDataProvider<P>,
    S: SealingPolicy<P>,
    BS: BatchStorage + ChunkStorage + ChunkWitnessStore,
    ES: ExecBlockStorage + Send + Sync,
{
    pub(crate) fn canonical_reader(&self) -> impl CanonicalChainReader {
        ExecChainCanonicalReader::new(self.exec_chain.clone(), self.block_storage.clone())
    }
}
