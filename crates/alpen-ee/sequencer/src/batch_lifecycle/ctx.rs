//! Context for the batch lifecycle task.

use std::sync::Arc;

use alpen_ee_common::{BatchDaProvider, BatchId, BatchProver, BatchStorage, DaBlobSource};
use alpen_reth_db::EeDaContext;
use tokio::sync::watch;

/// Context holding all dependencies for the batch lifecycle task.
///
/// This struct contains everything the task needs except for the mutable state,
/// which is passed separately to allow for state recovery on restart.
pub(crate) struct BatchLifecycleCtx<D, P, S>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    /// Receiver for new sealed batch notifications from batch_builder.
    pub sealed_batch_rx: watch::Receiver<BatchId>,

    /// Provider for posting and checking DA status.
    pub da_provider: Arc<D>,

    /// Provider for requesting and checking proof generation.
    pub prover: Arc<P>,

    /// Storage for batches.
    pub batch_storage: Arc<S>,

    /// Provider for DA blobs, also used to check per block state diff availability.
    pub blob_provider: Arc<dyn DaBlobSource>,

    /// Sender to notify about batches reaching ProofReady state.
    pub proof_ready_tx: watch::Sender<Option<BatchId>>,

    /// DA filter for cross-batch deduplication (bytecodes, extensible for addresses etc.).
    pub da_ctx: Arc<dyn EeDaContext + Send + Sync>,
}
