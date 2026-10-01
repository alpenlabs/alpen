use std::collections::BTreeMap;

#[cfg(feature = "native")]
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::OLSpecId;
use strata_predicate::PredicateKey;
#[cfg(feature = "native")]
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use zkaleido::ProgramId;
#[cfg(feature = "native")]
use zkaleido::ZkVmExecutor;
#[cfg(feature = "native")]
use zkaleido_native_adapter::NativeHost;

#[cfg(any(feature = "sp1", all(test, feature = "native")))]
use crate::RegistryError;

/// Holds a prewarmed program with its derived verification identity.
#[derive(Debug)]
pub struct CheckpointArtifact<H> {
    spec: OLSpecId,
    predicate: PredicateKey,
    program_id: ProgramId,
    host: H,
}

impl<H> CheckpointArtifact<H> {
    #[cfg(any(feature = "native", feature = "sp1"))]
    pub(crate) fn new(
        spec: OLSpecId,
        predicate: PredicateKey,
        program_id: ProgramId,
        host: H,
    ) -> Self {
        Self {
            spec,
            predicate,
            program_id,
            host,
        }
    }

    /// Returns the program's rules version.
    pub fn spec(&self) -> OLSpecId {
        self.spec
    }

    /// Returns the verifier predicate derived from the program.
    pub fn predicate(&self) -> &PredicateKey {
        &self.predicate
    }

    /// Returns the program identity derived during host setup.
    pub fn program_id(&self) -> &ProgramId {
        &self.program_id
    }

    /// Returns the initialized host.
    pub fn host(&self) -> &H {
        &self.host
    }
}

/// Stores immutable checkpoint artifacts indexed only by their OL rules version.
///
/// Contains only successfully loaded artifacts. Loading a configured bundle must succeed before
/// the registry can be used. The registry does not read files or mutate in response to ASM
/// transitions; operators configure additional bundles and restart to make them resident.
#[derive(Debug)]
pub struct CheckpointArtifactRegistry<H> {
    artifacts: BTreeMap<OLSpecId, CheckpointArtifact<H>>,
}

impl<H> CheckpointArtifactRegistry<H> {
    /// Creates a registry with no resident artifacts.
    pub fn empty() -> Self {
        Self {
            artifacts: BTreeMap::new(),
        }
    }

    #[cfg(any(feature = "sp1", all(test, feature = "native")))]
    pub(crate) fn insert(&mut self, artifact: CheckpointArtifact<H>) -> Result<(), RegistryError> {
        let spec = artifact.spec();
        if self.artifacts.contains_key(&spec) {
            return Err(RegistryError::DuplicateSpec { spec });
        }
        self.artifacts.insert(spec, artifact);
        Ok(())
    }

    /// Looks up the validated program for one epoch's spec.
    pub fn get(&self, spec: OLSpecId) -> Option<&CheckpointArtifact<H>> {
        self.artifacts.get(&spec)
    }

    /// Iterates over resident artifacts in spec order.
    pub fn iter(&self) -> impl Iterator<Item = &CheckpointArtifact<H>> {
        self.artifacts.values()
    }

    /// Transfers each resident host to its fixed-spec prover service.
    pub fn into_hosts(self) -> impl Iterator<Item = (OLSpecId, H)> {
        self.artifacts
            .into_iter()
            .map(|(spec, artifact)| (spec, artifact.host))
    }

    /// Copies the loaded specs and predicates without their proving hosts.
    pub fn to_predicates(&self) -> LoadedCheckpointPredicates {
        LoadedCheckpointPredicates {
            predicates: self
                .artifacts
                .iter()
                .map(|(&spec, artifact)| (spec, artifact.predicate.clone()))
                .collect(),
        }
    }
}

/// Maps each loaded [`OLSpecId`] to its validated predicate.
///
/// Artifact checks retain this metadata after the hosts move into prover services.
#[derive(Debug, Clone)]
pub struct LoadedCheckpointPredicates {
    predicates: BTreeMap<OLSpecId, PredicateKey>,
}

impl LoadedCheckpointPredicates {
    /// Returns the validated predicate for a spec, if its artifact is resident.
    pub fn predicate(&self, spec: OLSpecId) -> Option<&PredicateKey> {
        self.predicates.get(&spec)
    }

    /// Iterates over validated predicates in spec order.
    pub fn iter(&self) -> impl Iterator<Item = (OLSpecId, &PredicateKey)> {
        self.predicates
            .iter()
            .map(|(&spec, predicate)| (spec, predicate))
    }
}

/// Initializes the supported native checkpoint program for functional-test deployments.
///
/// Native programs share a test signing key, so their predicate alone cannot establish the
/// rules they execute. Each entry binds its spec to the corresponding program closure.
/// This binary implements only V1 proving; it never synthesizes programs for unknown specs.
#[cfg(feature = "native")]
pub fn native_checkpoint_registry(
    runtime_params: OLRuntimeParams,
) -> CheckpointArtifactRegistry<NativeHost> {
    let spec = OLSpecId::V1;
    let host = CheckpointProgram::native_host(spec, runtime_params);
    let artifact = CheckpointArtifact::new(
        spec,
        CheckpointProgram::test_predicate_key(),
        host.program_id(),
        host,
    );
    CheckpointArtifactRegistry {
        artifacts: BTreeMap::from([(spec, artifact)]),
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_types::OLSpecId;

    use super::{CheckpointArtifact, CheckpointArtifactRegistry, native_checkpoint_registry};
    use crate::RegistryError;

    #[test]
    fn missing_spec_does_not_fall_back_to_resident_program() {
        let registry = native_checkpoint_registry(OLRuntimeParams::test_default());
        assert_eq!(registry.get(OLSpecId::V1).unwrap().spec(), OLSpecId::V1);
        assert!(registry.get(OLSpecId::V0).is_none());
        assert!(registry.to_predicates().predicate(OLSpecId::V0).is_none());
    }

    #[test]
    fn registry_lookup_is_by_spec_even_when_predicates_repeat() {
        let native = native_checkpoint_registry(OLRuntimeParams::test_default());
        let source = native.get(OLSpecId::V1).unwrap();
        let mut registry = CheckpointArtifactRegistry::empty();
        // Unit hosts test the map's routing without claiming these are executable V0 programs.
        for spec in [OLSpecId::V0, OLSpecId::V1] {
            registry
                .insert(CheckpointArtifact::new(
                    spec,
                    source.predicate().clone(),
                    source.program_id().clone(),
                    u32::from(spec),
                ))
                .unwrap();
        }
        let duplicate = CheckpointArtifact::new(
            OLSpecId::V1,
            source.predicate().clone(),
            source.program_id().clone(),
            99,
        );
        assert!(matches!(
            registry.insert(duplicate),
            Err(RegistryError::DuplicateSpec { spec: OLSpecId::V1 })
        ));
        assert_eq!(*registry.get(OLSpecId::V0).unwrap().host(), 0);
        assert_eq!(*registry.get(OLSpecId::V1).unwrap().host(), 1);
    }
}
