//! Prover artifact validation helpers.

#[cfg(not(feature = "sp1"))]
mod no_sp1;
#[cfg(feature = "sp1")]
mod sp1;

pub(crate) use backend::CheckpointArtifactError;
#[cfg(not(feature = "sp1"))]
use no_sp1 as backend;
#[cfg(feature = "sp1")]
use sp1 as backend;
use strata_config::ProverConfig;
use strata_ol_params::OLRuntimeParams;
use tokio::runtime::Handle;

pub(crate) fn checkpoint_sp1_program_id(
    prover_config: &ProverConfig,
    handle: &Handle,
) -> Result<[u8; 32], CheckpointArtifactError> {
    backend::checkpoint_program_id(prover_config, handle)
}

pub(crate) fn validate_checkpoint_sp1_artifacts(
    runtime_params: OLRuntimeParams,
    program_id: &[u8; 32],
) -> Result<(), CheckpointArtifactError> {
    backend::validate_checkpoint_runtime_params_manifest(runtime_params, program_id)
}
