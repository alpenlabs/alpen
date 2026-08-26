//! Context provider for infra.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;

/// Context trait for a chain executor.  This should only be used by one
/// executor at a time.
pub trait ExecutorContext {
    fn store_processor_output(&self, id: ProcId) -> anyhow::Result<()>;
}

/// Context handed to a processor stage while it processes a link.
///
/// Borrows the executor's artifact cache and resolves dep fetches against the
/// link being processed and the link we arrived at its origin node by.
pub struct ProcContextImpl<'c, P: GChainProc> {
    cache: &'c ArtifactCache<P::Spec>,
    cur_lref: LinkRef<P::Spec>,
    prev_lref: Option<LinkRef<P::Spec>>,
}

impl<'c, P: GChainProc> ProcContextImpl<'c, P> {
    pub fn new(
        cache: &'c ArtifactCache<P::Spec>,
        cur_lref: LinkRef<P::Spec>,
        prev_lref: Option<LinkRef<P::Spec>>,
    ) -> Self {
        Self {
            cache,
            cur_lref,
            prev_lref,
        }
    }
}

impl<P: GChainProc> ProcContext<P> for ProcContextImpl<'_, P> {
    fn get_cur_artifact<A: ProcArtifact>(&self, proc_id: ProcId) -> Option<Arc<A>> {
        self.cache.get_artifact(&self.cur_lref, proc_id)
    }

    fn get_prev_artifact<A: ProcArtifact>(&self, proc_id: ProcId) -> Option<Arc<A>> {
        self.cache.get_artifact(self.prev_lref.as_ref()?, proc_id)
    }
}
