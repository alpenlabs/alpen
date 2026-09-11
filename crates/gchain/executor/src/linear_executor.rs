//! Simple single-threaded executor.
//!
//! Executes stages in-order, one-by-one.

#![expect(dead_code, reason = "executor is still being built out")]

use std::collections::*;
use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::errors::GExecError;
use crate::process::*;

/// Pipeline of multiple processor stages with associated scheduling information.
///
/// The stages are held in canonical order and `by_id` only indexes into them,
/// so there's no second copy of the ordering that could disagree with the first.
struct StagePipeline<S: GChainSpec> {
    stages: Vec<Stage<S>>,
    by_id: BTreeMap<ProcId, usize>,
}

impl<S: GChainSpec> StagePipeline<S> {
    /// Builds a pipeline from stages given in canonical order.
    ///
    /// Rejects duplicate IDs, since a dep naming a repeated ID wouldn't say
    /// which stage's output it meant.
    fn new(stages: Vec<Stage<S>>) -> Result<Self, GExecError> {
        let mut by_id = BTreeMap::new();
        for (idx, stage) in stages.iter().enumerate() {
            let proc_id = stage.chain_proc().proc_id();
            if by_id.insert(proc_id, idx).is_some() {
                return Err(GExecError::DuplicateProc(proc_id));
            }
        }

        Ok(Self { stages, by_id })
    }

    /// Returns an iterator over the stages in canonical order.
    fn iter_stages(&self) -> impl Iterator<Item = &Stage<S>> {
        self.stages.iter()
    }

    /// Looks up a stage by the ID it's registered under.
    fn get_stage(&self, proc_id: ProcId) -> Option<&Stage<S>> {
        self.by_id.get(&proc_id).map(|idx| &self.stages[*idx])
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
    fn fetch_link(&self, lref: &LinkRef<S>) -> Result<Link<S>, GExecError> {
        self.chain_provider
            .fetch_link(lref)?
            .ok_or_else(|| GExecError::MissingLink(format!("{lref:?}")))
    }

    /// Fetches the nodes a link connects, which is how the executor knows where
    /// the link sits relative to the path it's building.
    fn fetch_link_endpoints(&self, lref: &LinkRef<S>) -> Result<LinkEndpoints<S>, GExecError> {
        self.chain_provider
            .fetch_link_endpoints(lref)?
            .ok_or_else(|| GExecError::MissingLinkEndpoints(format!("{lref:?}")))
    }

    /// Executes all stages a single link.
    fn execute_link(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        let _link = self.fetch_link(lref)?;

        for _stage in self.pipeline.iter_stages() {
            // TODO
        }

        Ok(())
    }

    fn commit_link(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        let _link = self.fetch_link(lref)?;

        for _stage in self.pipeline.iter_stages() {
            // TODO
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::test_support::*;

    fn stage(proc_id: &str) -> Stage<TestSpec> {
        let proc_id = ProcId::from_str(proc_id).expect("test: parse ProcId");
        Stage {
            chain_proc: Arc::new(ProcShim::new(proc_id, TestProc)),
            deps: ProcDeps::new(Vec::new(), Vec::new()),
        }
    }

    fn proc_ids(pipeline: &StagePipeline<TestSpec>) -> Vec<ProcId> {
        pipeline
            .iter_stages()
            .map(|s| s.chain_proc().proc_id())
            .collect()
    }

    #[test]
    fn test_iter_stages_follows_canonical_order() {
        let pipeline = StagePipeline::new(vec![stage("third"), stage("first"), stage("second")])
            .expect("test: build pipeline");

        let expected = ["third", "first", "second"]
            .map(|s| ProcId::from_str(s).expect("test: parse ProcId"))
            .to_vec();
        assert_eq!(proc_ids(&pipeline), expected);
    }

    #[test]
    fn test_get_stage_finds_stage_by_id() {
        let pipeline = StagePipeline::new(vec![stage("first"), stage("second")])
            .expect("test: build pipeline");

        let found = pipeline
            .get_stage(ProcId::from_str("second").expect("test: parse ProcId"))
            .expect("test: find stage");
        assert_eq!(found.chain_proc().proc_id().as_ref(), "second");

        assert!(
            pipeline
                .get_stage(ProcId::from_str("absent").expect("test: parse ProcId"))
                .is_none()
        );
    }

    /// A dep naming a repeated ID wouldn't say which stage's output it meant.
    #[test]
    fn test_duplicate_proc_ids_are_rejected() {
        let err = StagePipeline::new(vec![stage("dup"), stage("other"), stage("dup")])
            .err()
            .expect("test: expected duplicate to be rejected");

        assert!(matches!(err, GExecError::DuplicateProc(id) if id.as_ref() == "dup"));
    }
}
