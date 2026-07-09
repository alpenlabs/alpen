use std::any::TypeId;
use std::collections::*;
use std::sync::Arc;

use strata_gchain_types::*;

/// Cached output from nodes that we've extracted and determined might be useful
/// for later proc stages.
pub struct ArtifactCache<S: GChainSpec> {
    nodes: HashMap<LinkRef<S>, BTreeMap<TypeId, Arc<dyn ProcArtifact>>>,
}

impl<S: GChainSpec> ArtifactCache<S> {
    /// Gets the stored output from some processor for some node.
    pub fn get_proc_artifact_arc<A: ProcArtifact>(
        &self,
        lref: &LinkRef<S>,
    ) -> Option<&Arc<dyn ProcArtifact>> {
        self.nodes
            .get(lref)
            .and_then(|atbl| atbl.get(&TypeId::of::<A>()))
    }

    pub fn get_proc_artifact<A: ProcArtifact>(&self, _lref: &LinkRef<S>) -> Option<&A> {
        // TODO(trey): need more complicated type hacks to make this work
        unimplemented!()
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
