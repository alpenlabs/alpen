//! Service input messages for OL checkpoint builder.

use strata_primitives::epoch::EpochCommitment;

/// Input messages for the OL checkpoint service.
#[derive(Debug)]
pub enum OLCheckpointMessage {
    /// New epoch summary commitment is available.
    NewEpochSummary(EpochCommitment),
    /// Input channel closed.
    Abort,
}
