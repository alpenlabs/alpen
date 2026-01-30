//! Provider traits for checkpoint building extensibility.
//!
//! These traits allow swapping implementations when transitioning from v1
//! (full state snapshot, empty logs, placeholder proof) to production
//! (DA diff, aggregated logs, ZK proof).

use strata_checkpoint_types::EpochSummary;
use strata_ol_chain_types_new::OLLog;

use crate::errors::WorkerResult;

/// Provides state diff data for checkpoint sidecar.
///
/// V1 implementation returns empty bytes.
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

/// V1 DA provider that returns empty bytes.
///
/// Production will use DA accumulation which tracks changes during execution.
#[derive(Debug)]
pub struct EmptyDaProvider;

impl DaProvider for EmptyDaProvider {
    fn compute_state_data(&self, _summary: &EpochSummary) -> WorkerResult<Vec<u8>> {
        Ok(Vec::new())
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
