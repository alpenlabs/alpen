use std::io;
use std::path::PathBuf;

use serde_json::Error as JsonError;
use strata_ol_state_types::OLSpecId;
#[cfg(feature = "sp1")]
use strata_proofimpl_predicate_keys::PredicateKeyError;
use thiserror::Error;
#[cfg(feature = "sp1")]
use tokio::task::JoinError;

/// Reports configuration or artifact-loading failures that prevent registry initialization.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// SP1 proving requires at least one explicitly configured artifact bundle.
    #[error(
        "SP1 proving requires at least one checkpoint artifact bundle with an explicit spec and bundle directory"
    )]
    NoArtifactsConfigured,
    /// A spec has more than one configured program.
    #[error("checkpoint artifacts configure OL spec {spec:?} more than once")]
    DuplicateSpec { spec: OLSpecId },
    /// A configured bundle could not be loaded or validated.
    #[error("failed to load checkpoint artifact for OL spec {spec:?} from {bundle_dir}: {source}")]
    LoadFailed {
        spec: OLSpecId,
        bundle_dir: PathBuf,
        source: ArtifactError,
    },
}

/// Describes why a configured artifact cannot be used.
#[derive(Debug, Error)]
pub enum ArtifactError {
    /// A configured spec is not supported for checkpoint proving.
    #[error("checkpoint proving does not support OL spec {spec:?}")]
    UnsupportedSpec { spec: OLSpecId },
    /// A bundle file could not be read.
    #[error("failed to read checkpoint artifact {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    /// A manifest could not be decoded.
    #[error("failed to parse checkpoint artifact manifest {path}: {source}")]
    ParseManifest { path: PathBuf, source: JsonError },
    /// A declared predicate could not be decoded.
    #[error("failed to parse checkpoint predicate {path}: {source}")]
    ParsePredicate { path: PathBuf, source: JsonError },
    /// A manifest uses an unsupported schema.
    #[error("unsupported checkpoint artifact manifest schema {schema}")]
    UnsupportedManifestSchema { schema: u32 },
    /// The manifest and registry disagree about the program's rules version.
    #[error(
        "checkpoint manifest declares spec {actual}, but its registry entry declares {expected:?}"
    )]
    SpecMismatch { expected: OLSpecId, actual: u32 },
    /// The runtime parameters do not match those declared for the built program.
    #[error(
        "checkpoint runtime params hash mismatch: expected {expected}, manifest declares {actual}"
    )]
    RuntimeParamsMismatch { expected: String, actual: String },
    /// The loaded ELF does not have the manifest's program ID.
    #[error("checkpoint program ID mismatch: ELF derives {expected}, manifest declares {actual}")]
    ProgramIdMismatch { expected: String, actual: String },
    /// The declared predicate does not match the ELF-derived verifier.
    #[cfg(feature = "sp1")]
    #[error(transparent)]
    Predicate(#[from] PredicateKeyError),
    /// Host setup failed within the isolated blocking task.
    #[cfg(feature = "sp1")]
    #[error("checkpoint host setup failed: {source}")]
    HostSetup { source: JoinError },
}
