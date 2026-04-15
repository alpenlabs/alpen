//! Replay-stage CLI error mapping.

use alpen_ee_common::DaBlob;
use ee_da_replay::{ReplayError, ReplaySummary};
use strata_cli_common::errors::DisplayedError;

/// Replays decoded DA blobs into execution state.
pub(crate) fn replay_blobs(
    chain_spec: &str,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, DisplayedError> {
    ee_da_replay::replay_blobs(chain_spec, blobs).map_err(|error| match error {
        ReplayError::InvalidChainSpec { .. } => {
            DisplayedError::UserError("invalid chain specification".to_string(), Box::new(error))
        }
        _ => {
            DisplayedError::InternalError("failed to replay DA blobs".to_string(), Box::new(error))
        }
    })
}
