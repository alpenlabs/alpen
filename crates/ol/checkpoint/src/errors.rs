//! Error types for OL checkpoint builder.

use strata_identifiers::OLBlockCommitment;
use strata_primitives::epoch::EpochCommitment;
use thiserror::Error;

/// Transient errors indicating checkpoint data is not ready yet.
#[derive(Debug, Error)]
pub(crate) enum CheckpointNotReady {
    /// No commitment found for the given epoch index.
    #[error("no commitment found for epoch index {0}")]
    EpochCommitment(u64),

    /// Missing epoch summary for the given commitment.
    #[error("missing summary for epoch commitment {0:?}")]
    EpochSummary(EpochCommitment),

    /// Missing terminal block header for the expected terminal commitment.
    #[error("missing terminal block header for commitment {0:?}")]
    TerminalBlock(OLBlockCommitment),
}
