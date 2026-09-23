//! Context provider for infra.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::errors::GExecError;

/// Context trait for a chain executor.  This should only be used by one
/// executor at a time.
///
/// Artifacts are persisted by the executor rather than by the stages that
/// produced them, so this is where the encoded artifact and the version of the
/// stage that produced it are handed off.
pub trait ExecutorContext {
    /// The chain spec this context stores artifacts for.
    type Spec: GChainSpec;

    /// Persists the artifact a stage produced for a link.
    fn store_processor_output(
        &self,
        lref: &LinkRef<Self::Spec>,
        proc_id: ProcId,
        data: &ProcessorArtifactData,
    ) -> Result<(), GExecError>;

    /// Loads the artifact a stage previously produced for a link, if it's still
    /// stored.
    ///
    /// The caller checks [`ProcessorArtifactData::exec_version`] against the
    /// stage's current version before trusting the contents.
    fn load_processor_output(
        &self,
        lref: &LinkRef<Self::Spec>,
        proc_id: ProcId,
    ) -> Result<Option<ProcessorArtifactData>, GExecError>;

    /// Discards a stored artifact, such as when the link it belongs to is
    /// pruned.
    fn discard_processor_output(
        &self,
        lref: &LinkRef<Self::Spec>,
        proc_id: ProcId,
    ) -> Result<(), GExecError>;
}

/// Context handed to a processor stage while it processes a link.
///
/// Borrows the executor's artifact cache and resolves dep fetches against the
/// link being processed and the uncommitted path leading to its origin node.
pub struct ProcContextImpl<'c, P: GChainProc> {
    cache: &'c ArtifactCache<P::Spec>,

    /// The path from the committed node to the origin of the link being
    /// processed, so its terminal node is that origin.
    path: &'c LinkPath<P::Spec>,

    cur_lref: LinkRef<P::Spec>,
    proc_id: ProcId,
}

impl<'c, P: GChainProc> ProcContextImpl<'c, P> {
    pub fn new(
        cache: &'c ArtifactCache<P::Spec>,
        path: &'c LinkPath<P::Spec>,
        cur_lref: LinkRef<P::Spec>,
        proc_id: ProcId,
    ) -> Self {
        Self {
            cache,
            path,
            cur_lref,
            proc_id,
        }
    }
}

impl<P: GChainProc> ProcContext<P> for ProcContextImpl<'_, P> {
    fn proc_id(&self) -> ProcId {
        self.proc_id
    }

    fn get_cur_artifact<A: ProcArtifact>(&self, proc_id: ProcId) -> Option<Arc<A>> {
        self.cache.get_artifact(&self.cur_lref, proc_id)
    }

    fn get_prev_artifact<A: ProcArtifact>(&self, proc_id: ProcId) -> Option<Arc<A>> {
        self.cache.get_artifact(self.path.links().last()?, proc_id)
    }

    fn get_path_artifacts<A: ProcArtifact>(
        &self,
        proc_id: ProcId,
    ) -> Option<PathArtifacts<P::Spec, A>> {
        let steps = self
            .path
            .links()
            .iter()
            .map(|lref| Some((lref.clone(), self.cache.get_artifact::<A>(lref, proc_id)?)))
            .collect::<Option<Vec<_>>>()?;
        Some(PathArtifacts::new(self.path.base_node().clone(), steps))
    }
}
