//! Epoch reconstruction from checkpoint DA.
//!
//! These operations take the encoded DA diff, so each rule set decodes it with
//! the DA encoding its epochs use.

use strata_codec::CodecError;
use strata_ol_chain_types_v1::AsmManifest;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::{IStateAccessorMut, OLSpecId};
use strata_ol_stf_v1::{EpochExecExpectations, EpochInfo, ExecError};
use thiserror::Error;

use crate::spec::{OLStfSpec, dispatch_spec};
use crate::v1::StfV1;

/// Errors produced while reconstructing an epoch from its encoded DA diff.
#[derive(Debug, Error)]
pub enum EpochDaReplayError {
    /// The encoded DA diff does not decode under the epoch's DA scheme.
    #[error("decode OL DA payload: {0}")]
    Decode(#[source] CodecError),

    /// Applying the diff or replaying the epoch's manifests failed.
    #[error("epoch DA replay: {0}")]
    Exec(#[source] ExecError),
}

/// Applies an epoch's encoded DA diff and replays its L1 manifests under `spec`
/// without checking the resulting state root.
///
/// Use this only when an upstream proof already binds the diff to the epoch's
/// post-state root. See [`strata_ol_stf_v1::apply_da_epoch`].
///
/// # Errors
///
/// Returns [`EpochDaReplayError::Decode`] if `encoded_diff` does not decode and
/// [`EpochDaReplayError::Exec`] if applying it or replaying the manifests fails.
pub fn apply_da_epoch<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    epoch_info: &EpochInfo,
    encoded_diff: &[u8],
    manifests: &[AsmManifest],
    runtime_params: &OLRuntimeParams,
) -> Result<(), EpochDaReplayError> {
    dispatch_spec!(spec => apply_da_epoch(
        state,
        epoch_info,
        encoded_diff,
        manifests,
        runtime_params,
    ))
}

/// Applies an epoch's encoded DA diff and replays its L1 manifests under
/// `spec`, then checks the resulting state root against `exp`.
///
/// See [`strata_ol_stf_v1::verify_epoch_with_diff`].
///
/// # Errors
///
/// Returns [`EpochDaReplayError::Decode`] if `encoded_diff` does not decode and
/// [`EpochDaReplayError::Exec`] if replay fails or the state root differs.
pub fn verify_epoch_with_diff<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    epoch_info: &EpochInfo,
    encoded_diff: &[u8],
    manifests: &[AsmManifest],
    exp: &EpochExecExpectations,
    runtime_params: &OLRuntimeParams,
) -> Result<(), EpochDaReplayError> {
    dispatch_spec!(spec => verify_epoch_with_diff(
        state,
        epoch_info,
        encoded_diff,
        manifests,
        exp,
        runtime_params,
    ))
}
