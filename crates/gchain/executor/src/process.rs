//! GChain processor indirection wrappers.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::context::ProcContextImpl;
use crate::errors::GExecError;

/// Dyn-compatible view of a [`GChainProc`].
///
/// The executor drives a heterogeneous pipeline of processor stages, so it can
/// name neither their concrete types nor their artifact types.  This trait
/// erases both, moving artifacts around as [`DynProcArtifact`] and downcasting
/// them back to the stage's own artifact type inside [`ProcShim`].
pub(crate) trait GChainProcDyn<S: GChainSpec>: 'static {
    /// The ID the wrapped processor stage is registered under.
    fn proc_id(&self) -> ProcId;

    /// See [`GChainProc::proc_version`].
    fn proc_version(&self) -> ProcVersion;

    /// See [`GChainProc::on_init`].
    fn on_init(&self, cur_node: &NodeRef<S>) -> Result<(), ProcError>;

    /// Processes a link and returns the type-erased artifact.
    ///
    /// The cache supplies the stage's declared deps, resolved against the link
    /// being processed and `path`, the uncommitted path from the committed node
    /// to the link's origin node.
    ///
    /// See [`GChainProc::process_link`].
    fn process_link(
        &self,
        lref: &LinkRef<S>,
        link: &Link<S>,
        cache: &ArtifactCache<S>,
        path: &LinkPath<S>,
    ) -> Result<Arc<dyn DynProcArtifact>, ProcError>;

    /// Decodes a persisted artifact back into the stage's artifact type.
    ///
    /// The caller checks the data's version against [`Self::proc_version`]
    /// first; this only decodes.
    fn decode_artifact(
        &self,
        data: &ProcessorArtifactData,
    ) -> Result<Arc<dyn DynProcArtifact>, ProcError>;

    /// See [`GChainProc::commit_outputs`].
    fn commit_outputs(
        &self,
        path: &LinkPath<S>,
        outputs: &[Arc<dyn DynProcArtifact>],
    ) -> Result<(), ProcError>;

    /// See [`GChainProc::uncommit_outputs`].
    fn uncommit_outputs(
        &self,
        path: &LinkPath<S>,
        outputs: &[Arc<dyn DynProcArtifact>],
    ) -> Result<(), ProcError>;

    /// See [`GChainProc::preprune_artifact`].
    fn preprune_artifact(
        &self,
        lref: &LinkRef<S>,
        artifact: &dyn DynProcArtifact,
    ) -> Result<(), ProcError>;

    /// See [`GChainProc::prune_state_upto`].
    fn prune_state_upto(&self, nref: &NodeRef<S>) -> Result<(), ProcError>;
}

/// Generic processor shim wrapper to expose as `dyn`-safe object.
pub(crate) struct ProcShim<P: GChainProc> {
    proc_id: ProcId,
    proc: P,
}

impl<P: GChainProc> ProcShim<P> {
    /// Wraps a processor stage under the ID the executor registers it as.
    ///
    /// [`PipelineBuilder`](crate::PipelineBuilder) is what pairs the two in
    /// practice, so the registered key and the shim's ID can't drift apart.
    pub(crate) fn new(proc_id: ProcId, proc: P) -> Self {
        Self { proc_id, proc }
    }
}

impl<S: GChainSpec, P: GChainProc<Spec = S>> GChainProcDyn<S> for ProcShim<P> {
    fn proc_id(&self) -> ProcId {
        self.proc_id
    }

    fn proc_version(&self) -> ProcVersion {
        self.proc.proc_version()
    }

    fn on_init(&self, cur_node: &NodeRef<S>) -> Result<(), ProcError> {
        self.proc.on_init(cur_node)
    }

    fn process_link(
        &self,
        lref: &LinkRef<S>,
        link: &Link<S>,
        cache: &ArtifactCache<S>,
        path: &LinkPath<S>,
    ) -> Result<Arc<dyn DynProcArtifact>, ProcError> {
        let ctx = ProcContextImpl::<P>::new(cache, path, lref.clone(), self.proc_id);
        let artifact = self.proc.process_link(lref, link, &ctx)?;
        Ok(Arc::new(artifact))
    }

    fn decode_artifact(
        &self,
        data: &ProcessorArtifactData,
    ) -> Result<Arc<dyn DynProcArtifact>, ProcError> {
        let artifact = data.try_decode_artifact::<P::Artifact>()?;
        Ok(Arc::new(artifact))
    }

    fn commit_outputs(
        &self,
        path: &LinkPath<S>,
        outputs: &[Arc<dyn DynProcArtifact>],
    ) -> Result<(), ProcError> {
        let outputs = downcast_artifacts::<P>(self.proc_id, outputs)?;
        self.proc.commit_outputs(path, &outputs)
    }

    fn uncommit_outputs(
        &self,
        path: &LinkPath<S>,
        outputs: &[Arc<dyn DynProcArtifact>],
    ) -> Result<(), ProcError> {
        let outputs = downcast_artifacts::<P>(self.proc_id, outputs)?;
        self.proc.uncommit_outputs(path, &outputs)
    }

    fn preprune_artifact(
        &self,
        lref: &LinkRef<S>,
        artifact: &dyn DynProcArtifact,
    ) -> Result<(), ProcError> {
        let artifact = artifact
            .as_any()
            .downcast_ref::<P::Artifact>()
            .ok_or_else(|| artifact_type_mismatch(self.proc_id))?;
        self.proc.preprune_artifact(lref, artifact)
    }

    fn prune_state_upto(&self, nref: &NodeRef<S>) -> Result<(), ProcError> {
        self.proc.prune_state_upto(nref)
    }
}

/// Reports artifacts having gotten crossed between stages inside the executor.
///
/// This is an executor bug rather than anything the stage did, but it surfaces
/// at the shim boundary where only a [`ProcError`] can be returned, so the
/// typed executor error is preserved as its source.
fn artifact_type_mismatch(proc_id: ProcId) -> ProcError {
    ProcError::custom(GExecError::ArtifactTypeMismatch(proc_id))
}

/// Recovers a stage's own artifact type from a type-erased artifact.
fn downcast_artifact<P: GChainProc>(
    proc_id: ProcId,
    artifact: &Arc<dyn DynProcArtifact>,
) -> Result<Arc<P::Artifact>, ProcError> {
    Arc::clone(artifact)
        .into_any_arc()
        .downcast::<P::Artifact>()
        .map_err(|_| artifact_type_mismatch(proc_id))
}

/// Recovers a stage's own artifact type across a run of type-erased artifacts,
/// preserving their order.
fn downcast_artifacts<P: GChainProc>(
    proc_id: ProcId,
    artifacts: &[Arc<dyn DynProcArtifact>],
) -> Result<Vec<Arc<P::Artifact>>, ProcError> {
    artifacts
        .iter()
        .map(|a| downcast_artifact::<P>(proc_id, a))
        .collect()
}
