//! Provider traits for checkpoint building extensibility.
//!
//! These traits allow swapping implementations when transitioning from v1
//! (full state snapshot, empty logs, placeholder proof) to production
//! (DA diff, aggregated logs, ZK proof).

use std::sync::Arc;

use ssz::Encode;
use strata_checkpoint_types::EpochSummary;
use strata_ol_chain_types_new::OLLog;
use strata_storage::OLStateManager;

use crate::errors::{OLCheckpointError, WorkerResult};

/// Provides state diff data for checkpoint sidecar.
///
/// V1 implementation returns full state snapshot.
/// Production implementation will compute DA diff from pre/post state.
pub trait DaProvider: Send + Sync {
    /// Compute the state representation for the given epoch.
    fn compute_state_data(&self, summary: &EpochSummary) -> WorkerResult<Vec<u8>>;
}

/// Provides OL logs for checkpoint sidecar.
///
/// V1 implementation returns empty logs.
/// Production implementation will aggregate logs from block execution.
pub trait LogProvider: Send + Sync {
    /// Get aggregated OL logs for the epoch.
    fn get_epoch_logs(&self, summary: &EpochSummary) -> WorkerResult<Vec<OLLog>>;
}

/// Provides proof bytes for checkpoint payload.
///
/// V1 implementation returns empty placeholder.
/// Production implementation will integrate with prover client.
pub trait ProofProvider: Send + Sync {
    /// Get proof bytes for the checkpoint.
    fn get_proof(&self, summary: &EpochSummary) -> WorkerResult<Vec<u8>>;
}

/// V1 DA provider that returns full OL state snapshot.
///
/// For v1, we return the full postseal state (final epoch state). This includes
/// L1 manifest changes which are technically redundant (already on L1), but is
/// acceptable for the "make it work" phase.
///
/// Production will use DA accumulation (PR #1310) which tracks changes during
/// execution and excludes post-seal writes automatically.
#[expect(
    missing_debug_implementations,
    reason = "OLStateManager doesn't implement Debug"
)]
pub struct FullStateDaProvider {
    ol_state: Arc<OLStateManager>,
}

impl FullStateDaProvider {
    /// Create a new full state DA provider.
    pub fn new(ol_state: Arc<OLStateManager>) -> Self {
        Self { ol_state }
    }
}

impl DaProvider for FullStateDaProvider {
    fn compute_state_data(&self, summary: &EpochSummary) -> WorkerResult<Vec<u8>> {
        let terminal = *summary.terminal();

        // Get postseal state (final epoch state after manifest sealing)
        // Note: For v1, we use postseal which includes L1 manifest changes.
        // Production DA (PR #1310) will use accumulation to exclude post-seal writes.
        let state = self
            .ol_state
            .get_toplevel_ol_state_blocking(terminal)?
            .ok_or(OLCheckpointError::MissingOLState(terminal))?;

        Ok(state.as_ssz_bytes())
    }
}

/// V1 log provider that returns empty logs.
///
/// This is a temporary implementation. Production will aggregate
/// logs from block execution storage.
#[derive(Debug)]
pub struct EmptyLogProvider;

impl LogProvider for EmptyLogProvider {
    fn get_epoch_logs(&self, _summary: &EpochSummary) -> WorkerResult<Vec<OLLog>> {
        Ok(Vec::new())
    }
}

/// V1 proof provider that returns empty placeholder.
///
/// This is a temporary implementation. Production will integrate
/// with the prover client for ZK proof generation.
#[derive(Debug)]
pub struct PlaceholderProofProvider;

impl ProofProvider for PlaceholderProofProvider {
    fn get_proof(&self, _summary: &EpochSummary) -> WorkerResult<Vec<u8>> {
        Ok(Vec::new())
    }
}
