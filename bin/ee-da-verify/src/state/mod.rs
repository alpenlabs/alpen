use alpen_ee_common::DaBlob;
use strata_cli_common::errors::DisplayedError;

pub(crate) mod replay;
#[cfg(test)]
pub(crate) mod test_utils;

pub(crate) use replay::{AppliedExecBlockRange, ReplaySummary};

/// Replays reassembled DA blobs into execution state.
pub(crate) fn replay_reassembled_blobs(
    chain_spec: &str,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, DisplayedError> {
    replay::replay_blobs(chain_spec, blobs).map_err(|error| match error {
        replay::ReplayError::InvalidChainSpec { .. } => {
            DisplayedError::UserError("invalid chain specification".to_string(), Box::new(error))
        }
        _ => {
            DisplayedError::InternalError("failed to replay DA blobs".to_string(), Box::new(error))
        }
    })
}
