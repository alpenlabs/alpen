//! Simple single-threaded executor.
//!
//! Executes stages in-order, one-by-one.

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
    canonical_order: Vec<ProcId>,
}

impl<S: GChainSpec> StagePipeline<S> {
    fn canonical_order(&self) -> &[ProcId] {
        &self.canonical_order
    }

    /// Returns an iterator over the stages in canonical order.
    fn iter_stages(&self) -> impl Iterator<Item = &Stage<S>> {
        self.canonical_order()
            .iter()
            .map(|id| self.stages.get(id).unwrap())
    }
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

struct ProcStateTackingTbl<S: GChainSpec> {
    proc_states: BTreeMap<ProcId, ProcTrackingState<S>>,
}

/// Tracks recent execution history about a node.
struct ProcTrackingState<S: GChainSpec> {
    committed_node: NodeRef<S>,
}

/// Linear processor pipeline executor.
///
/// This is still a "low initiative" data structure, it must be driven by some
/// external sync engine.
pub struct LinearExecutor<S: GChainSpec, P: ChainProvider<Spec = S>> {
    pipeline: Arc<StagePipeline<S>>,
    artifact_cache: ArtifactCache<S>,
    chain_provider: Arc<P>,
    tracking_tbl: ProcStateTackingTbl<S>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>> LinearExecutor<S, P> {
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
    fn execute_link(&mut self, lref: LinkRef<S>) -> anyhow::Result<()> {
        let link = self.fetch_link(lref)?;

        for stage in self.pipeline.iter_stages() {
            // TODO
        }

        Ok(())
    }

    fn commit_link(&mut self, lref: LinkRef<S>) -> anyhow::Result<()> {
        let link = self.fetch_link(lref)?;

        for stage in self.pipeline.iter_stages() {
            // TODO
        }

        Ok(())
    }
}
