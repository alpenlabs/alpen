use std::collections::*;
use std::sync::Arc;

use strata_gchain_types::*;

/// Cached artifacts from links that we've extracted and determined might be
/// useful for later proc stages.
///
/// Artifacts are keyed by the processor stage that produced them, since that's
/// how processor stages name their dependencies (see [`ProcDeps`]).  Multiple
/// stages may produce artifacts of the same concrete type.
pub struct ArtifactCache<S: GChainSpec> {
    links: HashMap<LinkRef<S>, BTreeMap<ProcId, Arc<dyn DynProcArtifact>>>,
}

impl<S: GChainSpec> ArtifactCache<S> {
    /// Creates a new empty cache.
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
        }
    }

    /// Stores the artifact a processor stage produced for a link, replacing any
    /// artifact that stage had already stored for it.
    pub fn insert_artifact(
        &mut self,
        lref: LinkRef<S>,
        proc_id: ProcId,
        artifact: Arc<dyn DynProcArtifact>,
    ) {
        self.links
            .entry(lref)
            .or_default()
            .insert(proc_id, artifact);
    }

    /// Gets the type-erased artifact some processor stage stored for a link.
    pub fn get_artifact_dyn(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<&Arc<dyn DynProcArtifact>> {
        self.links.get(lref).and_then(|atbl| atbl.get(&proc_id))
    }

    /// Gets the artifact some processor stage stored for a link, downcast to its
    /// concrete type.
    ///
    /// Returns `None` if the stage stored no artifact for the link, or if the
    /// artifact it stored isn't of type `A`.
    pub fn get_artifact<A: ProcArtifact>(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<Arc<A>> {
        let artifact = self.get_artifact_dyn(lref, proc_id)?;
        Arc::clone(artifact).into_any_arc().downcast::<A>().ok()
    }

    /// Discards every artifact stored for a link.
    pub fn remove_link(&mut self, lref: &LinkRef<S>) {
        self.links.remove(lref);
    }
}

impl<S: GChainSpec> Default for ArtifactCache<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// Context from the executor passed into a processor.
pub struct ProcContextImpl<P: GChainProc> {
    cached_outputs: ArtifactCache<P::Spec>,
}

impl<P: GChainProc> ProcContextImpl<P> {
    // TODO
}

pub struct ProcHistory<P: GChainProc> {
    base: NodeRef<P::Spec>,
    steps: Vec<Arc<ProcStepOutput<P>>>,
}

impl<P: GChainProc> ProcHistory<P> {
    pub fn new(base: NodeRef<P::Spec>, steps: Vec<Arc<ProcStepOutput<P>>>) -> Self {
        Self { base, steps }
    }

    pub fn new_base(base: NodeRef<P::Spec>) -> Self {
        Self::new(base, Vec::new())
    }

    /// Pushes a step onto the end of this processing history.
    pub fn push_step(&mut self, outp: Arc<ProcStepOutput<P>>) {
        self.steps.push(outp);
    }

    pub fn base(&self) -> &NodeRef<P::Spec> {
        &self.base
    }

    pub fn steps(&self) -> &[Arc<ProcStepOutput<P>>] {
        &self.steps
    }
}
