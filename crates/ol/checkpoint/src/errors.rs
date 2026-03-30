//! Error types for OL checkpoint builder.

use strata_identifiers::{Epoch, EpochCommitment};
use thiserror::Error;

/// Transient errors indicating checkpoint data is not ready yet.
#[derive(Debug, Error)]
pub(crate) enum CheckpointNotReady {
    /// No commitment found for the given epoch.
    #[error("no commitment found for epoch index {0}")]
    EpochCommitment(Epoch),

    /// Missing epoch summary for the given commitment.
    #[error("missing summary for epoch commitment {0:?}")]
    EpochSummary(EpochCommitment),
}
