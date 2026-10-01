use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use strata_ol_state_types::{OLSpecId, UnknownOLSpecId};

/// Identifies one checkpoint artifact bundle and the OL rules it proves.
///
/// Each directory contains `guest-checkpoint.elf`, `guest-checkpoint.predicate`, and
/// `guest-checkpoint.artifact-manifest.json`. Keeping these files together avoids combining an
/// ELF from one release with metadata from another.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "ArtifactConfigFields", into = "ArtifactConfigFields")]
pub struct CheckpointArtifactConfig {
    spec: OLSpecId,
    bundle_dir: PathBuf,
}

impl CheckpointArtifactConfig {
    /// Declares a bundle whose versioned manifest names `spec`.
    pub fn new(spec: OLSpecId, bundle_dir: PathBuf) -> Self {
        Self { spec, bundle_dir }
    }

    /// Returns the OL rules declared for the bundle.
    pub fn spec(&self) -> OLSpecId {
        self.spec
    }

    /// Returns the directory containing the complete bundle.
    pub fn bundle_dir(&self) -> &Path {
        &self.bundle_dir
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactConfigFields {
    spec: u32,
    bundle_dir: PathBuf,
}

impl TryFrom<ArtifactConfigFields> for CheckpointArtifactConfig {
    type Error = UnknownOLSpecId;

    fn try_from(fields: ArtifactConfigFields) -> Result<Self, Self::Error> {
        Ok(Self::new(
            OLSpecId::try_from(fields.spec)?,
            fields.bundle_dir,
        ))
    }
}

impl From<CheckpointArtifactConfig> for ArtifactConfigFields {
    fn from(config: CheckpointArtifactConfig) -> Self {
        Self {
            spec: u32::from(config.spec),
            bundle_dir: config.bundle_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CheckpointArtifactConfig;
    use strata_ol_state_types::OLSpecId;

    #[test]
    fn explicit_config_requires_a_known_numeric_spec() {
        let config: CheckpointArtifactConfig =
            toml::from_str("spec = 1\nbundle_dir = 'releases/v1'").unwrap();
        assert_eq!(config.spec(), OLSpecId::V1);
        assert!(
            toml::from_str::<CheckpointArtifactConfig>("spec = 2\nbundle_dir = 'releases/v2'")
                .is_err()
        );
    }
}
