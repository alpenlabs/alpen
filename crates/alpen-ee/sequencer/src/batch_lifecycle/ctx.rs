//! Context for the batch lifecycle task.

use std::sync::Arc;

use alpen_ee_common::{BatchDaProvider, BatchId, BatchProver, BatchStorage};
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

    /// Sender to notify about batches reaching ProofReady state.
    pub proof_ready_tx: watch::Sender<Option<BatchId>>,
}
