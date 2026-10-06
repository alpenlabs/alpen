//! Loads and validates SP1 checkpoint bundles before exposing their hosts.

use std::collections::BTreeSet;
use std::path::Path;
use std::{io, str};

use serde::Deserialize;
use serde::de::value::StrDeserializer;
use serde_json::Error as JsonError;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::OLSpecId;
use strata_predicate::PredicateKey;
use strata_proofimpl_predicate_keys::{
    PredicateKeyProvider, Sp1Groth16PredicateKey, validate_expected_predicate_key,
};
use tokio::runtime::Handle;
use tokio::{fs, task};
use zkaleido::ZkVmExecutor;
use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

use crate::manifest::ArtifactManifest;
use crate::{
    ArtifactError, CheckpointArtifact, CheckpointArtifactConfig, CheckpointArtifactRegistry,
    RegistryError,
};

const ELF_FILE: &str = "guest-checkpoint.elf";
const PREDICATE_FILE: &str = "guest-checkpoint.predicate";
const MANIFEST_FILE: &str = "guest-checkpoint.artifact-manifest.json";

/// Loads and validates every configured bundle before returning a resident registry.
///
/// Rejects empty configurations and duplicate specs before reading files or initializing hosts.
/// Any bundle failure aborts initialization with its cause, including artifacts not yet required
/// by ASM. Required ASM artifacts are checked separately after loading. Requires a Tokio runtime
/// with blocking-task support.
pub async fn load_checkpoint_registry(
    configs: &[CheckpointArtifactConfig],
    runtime_params: OLRuntimeParams,
    host_config: SP1HostConfig,
) -> Result<CheckpointArtifactRegistry<SP1Host>, RegistryError> {
    if configs.is_empty() {
        return Err(RegistryError::NoArtifactsConfigured);
    }
    let mut specs = BTreeSet::new();
    for config in configs {
        if !specs.insert(config.spec()) {
            return Err(RegistryError::DuplicateSpec {
                spec: config.spec(),
            });
        }
    }

    let mut registry = CheckpointArtifactRegistry::empty();
    let runtime_params_hash = runtime_params.hash();
    for config in configs {
        let artifact = load_artifact(config, runtime_params_hash, host_config.clone())
            .await
            .map_err(|source| RegistryError::LoadFailed {
                spec: config.spec(),
                bundle_dir: config.bundle_dir().to_owned(),
                source,
            })?;
        registry.insert(artifact)?;
    }
    Ok(registry)
}

async fn load_artifact(
    config: &CheckpointArtifactConfig,
    runtime_params_hash: [u8; 32],
    host_config: SP1HostConfig,
) -> Result<CheckpointArtifact<SP1Host>, ArtifactError> {
    if config.spec() == OLSpecId::V0 {
        return Err(ArtifactError::UnsupportedSpec {
            spec: config.spec(),
        });
    }
    let manifest_path = config.bundle_dir().join(MANIFEST_FILE);
    let manifest: ArtifactManifest =
        serde_json::from_slice(&read(&manifest_path).await?).map_err(|source| {
            ArtifactError::ParseManifest {
                path: manifest_path,
                source,
            }
        })?;
    manifest.validate_metadata(config, runtime_params_hash)?;
    let predicate_path = config.bundle_dir().join(PREDICATE_FILE);
    let predicate_bytes = read(&predicate_path).await?;
    let declared_predicate = parse_predicate(&predicate_bytes, &predicate_path)?;
    let elf = read(&config.bundle_dir().join(ELF_FILE)).await?;

    // The SDK does substantial setup work and its infallible initializer can panic on invalid
    // ELFs or backend failures. Isolate both from the async executor and retain the task error.
    let runtime = Handle::current();
    let (host, program_id) = task::spawn_blocking(move || {
        let host = runtime.block_on(SP1Host::init_with_config(&elf, host_config));
        let program_id = host.program_id();
        (host, program_id)
    })
    .await
    .map_err(|source| ArtifactError::HostSetup { source })?;
    manifest.validate_program_id(&program_id.0)?;
    let predicate = Sp1Groth16PredicateKey::new(program_id.0).predicate_key()?;
    validate_expected_predicate_key(&declared_predicate, &predicate)?;
    Ok(CheckpointArtifact::new(
        config.spec(),
        predicate,
        program_id,
        host,
    ))
}

async fn read(path: &Path) -> Result<Vec<u8>, ArtifactError> {
    fs::read(path).await.map_err(|source| ArtifactError::Read {
        path: path.to_owned(),
        source,
    })
}

fn parse_predicate(bytes: &[u8], path: &Path) -> Result<PredicateKey, ArtifactError> {
    // Reuse the protocol's human-readable predicate codec rather than maintaining a parser.
    let text = str::from_utf8(bytes).map_err(|source| ArtifactError::Read {
        path: path.to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })?;
    PredicateKey::deserialize(StrDeserializer::<JsonError>::new(text.trim())).map_err(|source| {
        ArtifactError::ParsePredicate {
            path: path.to_owned(),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::{env, fs};

    use serde_json::{Value, json};
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_types::OLSpecId;
    use strata_predicate::PredicateKey;
    use zkaleido_sp1_host::SP1HostConfig;

    use super::{
        ELF_FILE, MANIFEST_FILE, PREDICATE_FILE, load_checkpoint_registry, parse_predicate,
    };
    use crate::{ArtifactError, CheckpointArtifactConfig, RegistryError};

    #[tokio::test]
    async fn empty_configuration_is_rejected_without_loading_a_default_bundle() {
        let error = load_checkpoint_registry(
            &[],
            OLRuntimeParams::test_default(),
            SP1HostConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, RegistryError::NoArtifactsConfigured));
    }

    #[tokio::test]
    async fn missing_bundle_aborts_loading_with_spec_path_and_cause() {
        let dir = tempfile::tempdir().unwrap();
        let config = CheckpointArtifactConfig::new(OLSpecId::V1, dir.path().to_owned());
        let error = load_checkpoint_registry(
            &[config],
            OLRuntimeParams::test_default(),
            SP1HostConfig::default(),
        )
        .await
        .unwrap_err();
        let RegistryError::LoadFailed {
            spec,
            bundle_dir,
            source,
        } = error
        else {
            panic!("missing bundle must fail registry initialization");
        };
        assert_eq!(spec, OLSpecId::V1);
        assert_eq!(bundle_dir, dir.path());
        let ArtifactError::Read { path, source } = source else {
            panic!("missing manifest must preserve its read error");
        };
        assert_eq!(path, dir.path().join(MANIFEST_FILE));
        assert_eq!(source.kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn invalid_manifest_aborts_loading_before_host_initialization() {
        let dir = tempfile::tempdir().unwrap();
        let runtime_params = OLRuntimeParams::test_default();
        let mut wrong_hash = runtime_params.hash();
        wrong_hash[0] ^= 1;
        let mismatched = json!({
            "schema": 1,
            "spec": 1,
            "program_id": hex::encode([7; 32]),
            "runtime_params_hash": hex::encode(wrong_hash),
        })
        .to_string();
        for (manifest, malformed) in [("{", true), (mismatched.as_str(), false)] {
            fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
            let config = CheckpointArtifactConfig::new(OLSpecId::V1, dir.path().to_owned());
            let error =
                load_checkpoint_registry(&[config], runtime_params, SP1HostConfig::default())
                    .await
                    .unwrap_err();
            let RegistryError::LoadFailed { source, .. } = error else {
                panic!("invalid manifest must fail registry initialization");
            };
            if malformed {
                assert!(matches!(
                    source,
                    ArtifactError::ParseManifest { path, .. }
                        if path == dir.path().join(MANIFEST_FILE)
                ));
            } else {
                assert!(matches!(
                    source,
                    ArtifactError::RuntimeParamsMismatch { .. }
                ));
            }
        }
    }

    #[tokio::test]
    async fn duplicate_config_is_rejected_before_missing_bundle_reads() {
        let config = CheckpointArtifactConfig::new(OLSpecId::V1, PathBuf::from("absent"));
        let error = load_checkpoint_registry(
            &[config.clone(), config],
            OLRuntimeParams::test_default(),
            SP1HostConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            RegistryError::DuplicateSpec { spec: OLSpecId::V1 }
        ));
    }

    /// Runs only when a current guest bundle has been built for the supplied runtime params.
    #[tokio::test]
    #[ignore = "requires a freshly built SP1 bundle and SP1_PROVER=cpu"]
    async fn loads_real_bundle_and_rejects_tampered_identity() {
        assert_eq!(env::var("SP1_PROVER").as_deref(), Ok("cpu"));
        let bundle =
            PathBuf::from(env::var_os("CHECKPOINT_ARTIFACT_TEST_BUNDLE").expect("bundle path"));
        let params_path =
            env::var_os("CHECKPOINT_RUNTIME_PARAMS_PATH").expect("runtime params path");
        let runtime_params: OLRuntimeParams =
            serde_json::from_slice(&fs::read(params_path).unwrap()).unwrap();
        let config = CheckpointArtifactConfig::new(OLSpecId::V1, bundle.clone());
        let registry =
            load_checkpoint_registry(&[config], runtime_params, SP1HostConfig::default())
                .await
                .unwrap();
        let artifact = registry.get(OLSpecId::V1).unwrap();
        println!("loaded V1 checkpoint program {}", artifact.program_id());

        let tampered = tempfile::tempdir().unwrap();
        for filename in [ELF_FILE, MANIFEST_FILE, PREDICATE_FILE] {
            fs::copy(bundle.join(filename), tampered.path().join(filename)).unwrap();
        }
        let manifest_path = tampered.path().join(MANIFEST_FILE);
        let original_manifest = fs::read(&manifest_path).unwrap();
        for field in ["runtime_params_hash", "program_id"] {
            let mut manifest: Value = serde_json::from_slice(&original_manifest).unwrap();
            manifest[field] = Value::String(hex::encode([0; 32]));
            fs::write(&manifest_path, manifest.to_string()).unwrap();
            let config = CheckpointArtifactConfig::new(OLSpecId::V1, tampered.path().to_owned());
            let error =
                load_checkpoint_registry(&[config], runtime_params, SP1HostConfig::default())
                    .await
                    .unwrap_err();
            let RegistryError::LoadFailed { source, .. } = error else {
                panic!("tampered manifest must fail registry initialization");
            };
            match field {
                "runtime_params_hash" => assert!(matches!(
                    source,
                    ArtifactError::RuntimeParamsMismatch { .. }
                )),
                "program_id" => assert!(matches!(source, ArtifactError::ProgramIdMismatch { .. })),
                _ => unreachable!(),
            }
        }
        fs::write(&manifest_path, original_manifest).unwrap();
        fs::write(tampered.path().join(PREDICATE_FILE), "AlwaysAccept").unwrap();
        let config = CheckpointArtifactConfig::new(OLSpecId::V1, tampered.path().to_owned());
        let error = load_checkpoint_registry(&[config], runtime_params, SP1HostConfig::default())
            .await
            .unwrap_err();
        let RegistryError::LoadFailed { source, .. } = error else {
            panic!("tampered predicate must fail registry initialization");
        };
        assert!(matches!(source, ArtifactError::Predicate(_)));
    }

    #[test]
    fn predicate_sidecar_uses_protocol_codec() {
        assert_eq!(
            parse_predicate(b"AlwaysAccept\n", Path::new("predicate")).unwrap(),
            PredicateKey::always_accept()
        );
        assert!(parse_predicate(b"Sp1Groth16:badhex", Path::new("predicate")).is_err());
    }
}
