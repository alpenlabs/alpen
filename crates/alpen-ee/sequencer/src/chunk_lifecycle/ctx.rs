//! Context for the chunk proof lifecycle task.

use std::sync::Arc;

use alpen_ee_common::{BatchStorage, ChunkProver, ChunkStorage};

/// Dependencies shared across the chunk proof lifecycle, held once and threaded by reference.
///
/// The mutable cursor lives separately in
/// [`ChunkProofCursor`](super::state::ChunkProofCursor) so it can be recovered independently
/// on restart.
pub(super) struct ChunkLifecycleCtx<P, S>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    /// Prover used to request chunk proofs and check their status.
    pub prover: Arc<P>,

    /// Storage for chunk rows, their batch links, and batch status.
    pub storage: Arc<S>,
}
