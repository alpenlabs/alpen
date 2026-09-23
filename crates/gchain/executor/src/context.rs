//! Context handed to processor stages.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;

/// Context handed to a processor stage while it processes a link.
///
/// Borrows the executor's artifact cache and resolves dep fetches against the
/// link being processed and the uncommitted path leading to its origin node.
pub struct ProcContextImpl<'c, P: GChainProc> {
    cache: &'c ArtifactCache<P::Spec>,

    /// A path from the committed node to the origin of the link being
    /// processed, so its terminal node is that origin.  Which path it is when
    /// several converge on the origin is up to the executor.
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
