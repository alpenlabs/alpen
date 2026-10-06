//! Checkpoint proof artifact loading and per-spec lookup.
//!
//! Registry entries describe programs, never checkpoint activation boundaries. Node services
//! choose the epoch's spec from its committed OL start state before looking up an artifact.

mod config;
mod error;
#[cfg(any(feature = "sp1", test))]
mod manifest;
mod registry;
#[cfg(feature = "sp1")]
pub mod sp1;

pub use config::CheckpointArtifactConfig;
pub use error::{ArtifactError, RegistryError};
pub use registry::{CheckpointArtifact, CheckpointArtifactRegistry, LoadedCheckpointPredicates};

#[cfg(feature = "native")]
pub use registry::native_checkpoint_registry;

pub mod artifact_checks;
#[cfg(feature = "node")]
pub mod startup;

// Cargo makes every dev-dependency available to each test build, including when the feature
// owning its test suite is disabled. Acknowledge only those disabled suites' dependencies.
#[cfg(test)]
mod test_dependencies {
    #[cfg(not(any(feature = "native", feature = "sp1")))]
    use strata_ol_params as _;
    #[cfg(not(feature = "node"))]
    use strata_storage as _;
    #[cfg(not(feature = "sp1"))]
    use tempfile as _;
    #[cfg(not(all(feature = "node", feature = "native")))]
    use {strata_asm_checkpoint_types as _, strata_db_store_sled as _, strata_db_tests as _};
}
