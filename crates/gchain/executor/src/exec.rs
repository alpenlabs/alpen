use std::collections::*;
use std::sync::Arc;

use anyhow::Context;
use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::errors::GExecError;
use crate::process::*;

/// Pipeline of multiple processor stages with associated scheduling information.
struct StagePipeline<S: GChainSpec> {
    stages: BTreeMap<ProcId, Stage<S>>,
}

/// Description of a processor stage with associated exec control data.
struct Stage<S: GChainSpec> {
    chain_proc: Arc<dyn GChainProcDyn<S>>,
    deps: ProcDeps,
}

impl<S: GChainSpec> Stage<S> {
    fn chain_proc(&self) -> &dyn GChainProcDyn<S> {
        self.chain_proc.as_ref()
    }

    fn deps(&self) -> &ProcDeps {
        &self.deps
    }
}

/// Executor for a processor pipeline.
///
/// This is still a "low initiative" data structure, it must be driven by some
/// external sync engine.
pub struct Executor<S: GChainSpec, P: ChainProvider<Spec = S>> {
    pipeline: Arc<StagePipeline<S>>,
    artifact_cache: ArtifactCache<S>,
    chain_provider: Arc<P>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>> Executor<S, P> {
    /// Fetches a link from the underlying provider and repackages the errors to
    /// gobble missing links.
    fn fetch_link(&self, lref: LinkRef<S>) -> anyhow::Result<Link<S>> {
        self.chain_provider
            .fetch_link(&lref)
            .map_err(anyhow::Error::from)
            .and_then(|v| {
                v.ok_or(GExecError::MissingLink)
                    .map_err(anyhow::Error::from)
            })
            .with_context(|| format!("fetch link {lref:?}"))
    }

    /// Executes all stages a single link.
    fn execute_link(&self, lref: LinkRef<S>) -> anyhow::Result<()> {
        // TODO
        let link = self.fetch_link(lref)?;
        Ok(())
    }

    fn commit_link(&self, lref: LinkRef<S>) -> anyhow::Result<()> {
        // TODO
        let link = self.fetch_link(lref)?;
        Ok(())
    }
}
