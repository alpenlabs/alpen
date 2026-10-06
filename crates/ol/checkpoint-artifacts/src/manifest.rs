use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::{ArtifactError, CheckpointArtifactConfig};

/// Describes the release identity committed to an artifact bundle.
#[derive(Debug, Deserialize)]
pub(crate) struct ArtifactManifest {
    schema: u32,
    spec: u32,
    #[serde(deserialize_with = "deserialize_hash")]
    program_id: [u8; 32],
    #[serde(deserialize_with = "deserialize_hash")]
    runtime_params_hash: [u8; 32],
}

impl ArtifactManifest {
    pub(crate) fn validate_metadata(
        &self,
        config: &CheckpointArtifactConfig,
        expected_runtime_params_hash: [u8; 32],
    ) -> Result<(), ArtifactError> {
        if self.schema != 1 {
            return Err(ArtifactError::UnsupportedManifestSchema {
                schema: self.schema,
            });
        }
        if self.spec != u32::from(config.spec()) {
            return Err(ArtifactError::SpecMismatch {
                expected: config.spec(),
                actual: self.spec,
            });
        }
        if self.runtime_params_hash != expected_runtime_params_hash {
            return Err(ArtifactError::RuntimeParamsMismatch {
                expected: hex::encode(expected_runtime_params_hash),
                actual: hex::encode(self.runtime_params_hash),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_program_id(&self, derived: &[u8; 32]) -> Result<(), ArtifactError> {
        if derived != &self.program_id {
            return Err(ArtifactError::ProgramIdMismatch {
                expected: hex::encode(derived),
                actual: hex::encode(self.program_id),
            });
        }
        Ok(())
    }
}

fn deserialize_hash<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    let mut hash = [0; 32];
    hex::decode_to_slice(encoded, &mut hash).map_err(D::Error::custom)?;
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Error as JsonError, Value, json};
    use strata_ol_state_types::OLSpecId;

    use super::ArtifactManifest;
    use crate::{ArtifactError, CheckpointArtifactConfig};

    fn document() -> Value {
        json!({
            "schema": 1,
            "spec": 1,
            "program_id": hex::encode([7; 32]),
            "runtime_params_hash": hex::encode([8; 32]),
        })
    }

    fn parse(value: Value) -> Result<ArtifactManifest, JsonError> {
        serde_json::from_slice(value.to_string().as_bytes())
    }

    fn config() -> CheckpointArtifactConfig {
        CheckpointArtifactConfig::new(OLSpecId::V1, PathBuf::from("release"))
    }

    #[test]
    fn checks_schema_spec_runtime_params_and_derived_program_id() {
        let manifest = parse(document()).unwrap();
        manifest.validate_metadata(&config(), [8; 32]).unwrap();
        manifest.validate_program_id(&[7; 32]).unwrap();
        assert!(matches!(
            manifest.validate_metadata(&config(), [9; 32]),
            Err(ArtifactError::RuntimeParamsMismatch { .. })
        ));
        assert!(matches!(
            manifest.validate_program_id(&[6; 32]),
            Err(ArtifactError::ProgramIdMismatch { .. })
        ));

        let mut document = document();
        document["spec"] = json!(0);
        assert!(matches!(
            parse(document)
                .unwrap()
                .validate_metadata(&config(), [8; 32]),
            Err(ArtifactError::SpecMismatch { .. })
        ));
    }

    #[test]
    fn rejects_missing_version_unknown_schema_and_invalid_hashes() {
        let mut missing_spec = document();
        missing_spec.as_object_mut().unwrap().remove("spec");
        assert!(parse(missing_spec).is_err());

        let mut unknown_schema = document();
        unknown_schema["schema"] = json!(2);
        assert!(matches!(
            parse(unknown_schema)
                .unwrap()
                .validate_metadata(&config(), [8; 32]),
            Err(ArtifactError::UnsupportedManifestSchema { schema: 2 })
        ));

        for hash in ["not hex", "aa", &"aa".repeat(33)] {
            let mut invalid_hash = document();
            invalid_hash["program_id"] = json!(hash);
            assert!(parse(invalid_hash).is_err());
        }
    }
}
