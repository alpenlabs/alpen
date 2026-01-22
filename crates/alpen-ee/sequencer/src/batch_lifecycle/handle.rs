//! Handle and factory for the batch lifecycle task.

use std::{future::Future, sync::Arc};

use alpen_ee_common::{BatchDaProvider, BatchId, BatchProver, BatchStorage};
use tokio::sync::watch;

use super::{ctx::BatchLifecycleCtx, state::BatchLifecycleState, task::batch_lifecycle_task};

/// Handle to observe batch lifecycle state changes.
///
/// Provides a watch channel that is updated whenever a batch reaches ProofReady state.
#[derive(Debug, Clone)]
pub struct BatchLifecycleHandle {
    /// Receiver for batches that reach ProofReady state.
    latest_proof_ready_rx: watch::Receiver<Option<BatchId>>,
}

impl BatchLifecycleHandle {
    /// Returns a receiver that can be used to watch for proof-ready batch updates.
    pub fn latest_proof_ready_watcher(&self) -> watch::Receiver<Option<BatchId>> {
        self.latest_proof_ready_rx.clone()
    }

    /// Returns the current latest proof-ready batch ID.
    pub fn latest_proof_ready_batch(&self) -> Option<BatchId> {
        *self.latest_proof_ready_rx.borrow()
    }
}

/// Create batch lifecycle task.
pub fn create_batch_lifecycle_task<D, P, S>(
    initial_proof_ready_batch_id: Option<BatchId>,
    state: BatchLifecycleState,
    sealed_batch_rx: watch::Receiver<BatchId>,
    da_provider: Arc<D>,
    prover: Arc<P>,
    batch_storage: Arc<S>,
) -> (BatchLifecycleHandle, impl Future<Output = ()>)
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let (proof_ready_tx, proof_ready_rx) = watch::channel(initial_proof_ready_batch_id);

    let ctx = BatchLifecycleCtx {
        sealed_batch_rx,
        da_provider,
        prover,
        batch_storage,
        proof_ready_tx,
    };

    let handle = BatchLifecycleHandle {
        latest_proof_ready_rx: proof_ready_rx,
    };
    let task = batch_lifecycle_task(state, ctx);

    (handle, task)
}
