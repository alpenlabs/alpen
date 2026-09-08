//! SP1 prover artifact validation.

use std::{fs, io, path::Path};

use strata_config::ProverConfig;
use strata_ol_params::OLRuntimeParams;
use strata_zkvm_hosts::sp1::{checkpoint_host, checkpoint_runtime_params_manifest_path};
use thiserror::Error;
use tokio::runtime::Handle;

use crate::prover::checkpoint_sp1_host_config;

#[derive(Debug, Error)]
pub(crate) enum CheckpointArtifactError {
    #[error("failed to read checkpoint runtime params manifest from {path}: {source}")]
    ReadManifest { path: String, source: io::Error },

    #[error("failed to parse checkpoint runtime params manifest from {path}: {source}")]
    ParseManifest {
        path: String,
        source: serde_json::Error,
    },

    #[error("checkpoint runtime params manifest from {path} is missing {field}")]
    MissingManifestField { path: String, field: &'static str },

    #[error(
        "checkpoint runtime params manifest field {field} from {path} is invalid hex: {source}"
    )]
    InvalidManifestHex {
        path: String,
        field: &'static str,
        source: hex::FromHexError,
    },

    #[error(
        "checkpoint runtime params manifest field {field} from {path} has {actual_len} bytes; \
         expected 32"
    )]
    InvalidManifestHashLength {
        path: String,
        field: &'static str,
        actual_len: usize,
    },

    #[error(
        "checkpoint runtime params hash mismatch: loaded ol-params.json runtime hash {expected}, \
         but SP1 artifact manifest contains {actual}"
    )]
    RuntimeParamsHashMismatch { expected: String, actual: String },

    #[error(
        "checkpoint runtime params manifest program ID mismatch: loaded SP1 artifact has program \
         ID {expected}, but manifest contains {actual}"
    )]
    ProgramIdMismatch { expected: String, actual: String },
}

pub(crate) fn checkpoint_program_id(
    prover_config: &ProverConfig,
    handle: &Handle,
) -> Result<[u8; 32], CheckpointArtifactError> {
    use zkaleido::ZkVmExecutor;

    let sp1_config = checkpoint_sp1_host_config(prover_config);
    let host = handle.block_on(checkpoint_host(sp1_config));
    Ok(host.program_id().0)
}

pub(crate) fn validate_checkpoint_runtime_params_manifest(
    runtime_params: OLRuntimeParams,
    program_id: &[u8; 32],
) -> Result<(), CheckpointArtifactError> {
    let manifest_path = checkpoint_runtime_params_manifest_path();
    let manifest = read_runtime_params_manifest(&manifest_path)?;

    ensure_runtime_params_manifest_matches(runtime_params.hash(), program_id, &manifest)
}

struct CheckpointRuntimeParamsManifest {
    runtime_params_hash: [u8; 32],
    program_id: [u8; 32],
}

fn read_runtime_params_manifest(
    manifest_path: &Path,
) -> Result<CheckpointRuntimeParamsManifest, CheckpointArtifactError> {
    let manifest = fs::read_to_string(manifest_path).map_err(|source| {
        CheckpointArtifactError::ReadManifest {
            path: manifest_path.display().to_string(),
            source,
        }
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).map_err(|source| {
        CheckpointArtifactError::ParseManifest {
            path: manifest_path.display().to_string(),
            source,
        }
    })?;

    Ok(CheckpointRuntimeParamsManifest {
        runtime_params_hash: read_manifest_hash_field(
            &manifest,
            manifest_path,
            "runtime_params_hash",
        )?,
        program_id: read_manifest_hash_field(&manifest, manifest_path, "program_id")?,
    })
}

fn read_manifest_hash_field(
    manifest: &serde_json::Value,
    manifest_path: &Path,
    field: &'static str,
) -> Result<[u8; 32], CheckpointArtifactError> {
    let value = manifest
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CheckpointArtifactError::MissingManifestField {
            path: manifest_path.display().to_string(),
            field,
        })?;

    hex::decode(value)
        .map_err(|source| CheckpointArtifactError::InvalidManifestHex {
            path: manifest_path.display().to_string(),
            field,
            source,
        })?
        .try_into()
        .map_err(
            |bytes: Vec<u8>| CheckpointArtifactError::InvalidManifestHashLength {
                path: manifest_path.display().to_string(),
                field,
                actual_len: bytes.len(),
            },
        )
}

fn ensure_runtime_params_manifest_matches(
    expected_runtime_params_hash: [u8; 32],
    expected_program_id: &[u8; 32],
    manifest: &CheckpointRuntimeParamsManifest,
) -> Result<(), CheckpointArtifactError> {
    if manifest.runtime_params_hash != expected_runtime_params_hash {
        return Err(CheckpointArtifactError::RuntimeParamsHashMismatch {
            expected: hex::encode(expected_runtime_params_hash),
            actual: hex::encode(manifest.runtime_params_hash),
        });
    }
    if &manifest.program_id != expected_program_id {
        return Err(CheckpointArtifactError::ProgramIdMismatch {
            expected: hex::encode(expected_program_id),
            actual: hex::encode(manifest.program_id),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_runtime_params_hash() {
        let manifest = CheckpointRuntimeParamsManifest {
            runtime_params_hash: [1; 32],
            program_id: [2; 32],
        };
        let err = ensure_runtime_params_manifest_matches([3; 32], &[2; 32], &manifest).unwrap_err();

        assert!(matches!(
            err,
            CheckpointArtifactError::RuntimeParamsHashMismatch { .. }
        ));
        assert!(err.to_string().contains("runtime params hash mismatch"));
    }

    #[test]
    fn rejects_mismatched_runtime_params_manifest_program_id() {
        let manifest = CheckpointRuntimeParamsManifest {
            runtime_params_hash: [1; 32],
            program_id: [2; 32],
        };
        let err = ensure_runtime_params_manifest_matches([1; 32], &[3; 32], &manifest).unwrap_err();

        assert!(matches!(
            err,
            CheckpointArtifactError::ProgramIdMismatch { .. }
        ));
        assert!(err.to_string().contains("program ID mismatch"));
    }
}
