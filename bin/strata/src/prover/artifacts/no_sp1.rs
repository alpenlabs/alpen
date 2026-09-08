//! Fallbacks for builds without SP1 support.

use strata_config::ProverConfig;
use strata_ol_params::OLRuntimeParams;
use thiserror::Error;
use tokio::runtime::Handle;

#[derive(Debug, Error)]
pub(crate) enum CheckpointArtifactError {
    #[error("config.prover.backend=sp1 requires building `strata` with the `sp1` feature")]
    Sp1FeatureDisabled,
}

pub(crate) fn checkpoint_program_id(
    _prover_config: &ProverConfig,
    _handle: &Handle,
) -> Result<[u8; 32], CheckpointArtifactError> {
    Err(CheckpointArtifactError::Sp1FeatureDisabled)
}

pub(crate) fn validate_checkpoint_runtime_params_manifest(
    _runtime_params: OLRuntimeParams,
    _program_id: &[u8; 32],
) -> Result<(), CheckpointArtifactError> {
    Err(CheckpointArtifactError::Sp1FeatureDisabled)
}
