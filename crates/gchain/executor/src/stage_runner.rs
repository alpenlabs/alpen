//! Running the pipeline's stages and keeping what they produce.
//!
//! This is the execution half of an executor's bookkeeping: which artifacts
//! each stage has for which links, whether it still stands behind them or
//! only committed with them.  It never decides which links to run or which paths
//! to commit, and it does no store I/O: the executor hands it paths it worked
//! out from the provider, feeds it the stored artifacts those paths need,
//! and persists the artifacts a run hands back.  The stages' own side effects
//! (committing, pruning) do happen here, since driving them is the runner's
//! job.

use std::collections::HashSet;
use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::config::{Stage, StagePipeline};
use crate::errors::GExecError;
use crate::schedule::LinkCoverage;
use crate::store::ArtifactRecord;

/// The verdict on a link the executor was asked to process.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkOutcome {
    /// Every stage accepted the link; it's recorded and can be built on.
    Accepted,

    /// A stage rejected the link.  Nothing is recorded, and no later stage was
    /// run on it.
    Rejected { proc_id: ProcId },
}

/// What a run of the stages on a link came to, with the encoded artifacts
/// it produced for the executor to persist.
///
/// A rejected run produces nothing: the runner drops whatever the earlier
/// stages made before handing back the verdict.
pub(crate) struct StageRun<S: GChainSpec> {
    outcome: LinkOutcome,
    produced: Vec<ArtifactRecord<S>>,
}

impl<S: GChainSpec> StageRun<S> {
    pub(crate) fn outcome(&self) -> LinkOutcome {
        self.outcome
    }

    pub(crate) fn produced(&self) -> &[ArtifactRecord<S>] {
        &self.produced
    }
}

pub(crate) struct StageRunner<S: GChainSpec> {
    pipeline: StagePipeline<S>,
    cache: ArtifactCache<S>,

    /// Artifacts stored by versions of their stages that no longer stand for
    /// their links.  They don't count as artifacts for anything but rolling
    /// the stage back over what they committed.
    // TODO(trey): try to rework this so that it only exists in the reproc executor
    outdated: ArtifactCache<S>,
}

impl<S: GChainSpec> StageRunner<S> {
    pub(crate) fn new(pipeline: StagePipeline<S>) -> Self {
        Self {
            pipeline,
            cache: ArtifactCache::new(),
            outdated: ArtifactCache::new(),
        }
    }

    pub(crate) fn pipeline(&self) -> &StagePipeline<S> {
        &self.pipeline
    }

    pub(crate) fn get_artifact<A: ProcArtifact>(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<Arc<A>> {
        self.cache.get_artifact(lref, proc_id)
    }

    /// Whether taking in a stored artifact needs its link's header, which is
    /// what a stage judges an older version's artifact by.
    pub(crate) fn check_needs_header(&self, record: &ArtifactRecord<S>) -> bool {
        self.pipeline
            .get_stage(record.proc_id())
            .is_some_and(|stage| record.data().exec_version() < stage.chain_proc().proc_version())
    }

    /// Takes in a stored artifact from a stage still in the pipeline.
    ///
    /// It counts as the stage's artifact for the link if it came from the
    /// stage's current version or an older one the stage still accepts for
    /// the link.  Otherwise it's only kept for undoing what it committed, and
    /// not even that if it no longer decodes.  Without the header an older
    /// version's artifact is never accepted.
    pub(crate) fn insert_stored(
        &mut self,
        record: ArtifactRecord<S>,
        header: Option<&LinkHeader<S>>,
    ) -> Result<(), GExecError> {
        let (lref, proc_id, data) = record.into_parts();
        let Some(stage) = self.pipeline.get_stage(proc_id) else {
            return Ok(());
        };
        let chain_proc = stage.chain_proc();

        let stored = data.exec_version();
        let current = chain_proc.proc_version();
        if stored > current {
            return Err(GExecError::artifact_from_newer_version(&lref, proc_id));
        }

        let acceptable = stored == current
            || header.is_some_and(|h| chain_proc.check_artifact_acceptable(stored, &lref, h));
        if acceptable {
            let artifact = chain_proc
                .decode_artifact(&data)
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            self.cache.insert_artifact(lref, proc_id, artifact);
        } else if let Ok(artifact) = chain_proc.decode_artifact(&data) {
            self.outdated.insert_artifact(lref, proc_id, artifact);
        }
        Ok(())
    }

    /// The stages with no current artifact for a link, in canonical order.
    /// Every stage when nothing is cached for it.
    pub(crate) fn missing_stages(&self, lref: &LinkRef<S>) -> Vec<ProcId> {
        self.pipeline
            .proc_ids()
            .filter(|proc_id| !self.has_current(lref, *proc_id))
            .collect()
    }

    /// Whether a stage has a current artifact cached for a link.
    pub(crate) fn has_current(&self, lref: &LinkRef<S>, proc_id: ProcId) -> bool {
        self.cache.get_artifact_dyn(lref, proc_id).is_some()
    }

    /// Drops the cached artifacts of every link outside a set.
    pub(crate) fn retain_links(&mut self, keep: &HashSet<LinkRef<S>>) {
        self.cache.retain_links(keep);
        self.outdated.retain_links(keep);
    }

    /// Runs some stages on a link in canonical order, caching each artifact
    /// as it's produced, until one rejects the link.
    ///
    /// A rejection drops what this run produced, leaving artifacts from
    /// earlier runs for the caller to deal with.
    pub(crate) fn run(
        &mut self,
        lref: &LinkRef<S>,
        link: &Link<S>,
        path: &LinkPath<S>,
        stages: &[ProcId],
    ) -> Result<StageRun<S>, GExecError> {
        let mut produced: Vec<ArtifactRecord<S>> = Vec::new();
        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            if !stages.contains(&proc_id) {
                continue;
            }

            let coverage = self.link_coverage(lref, path);
            self.pipeline.schedule().check_ready(proc_id, &coverage)?;

            let artifact = stage
                .chain_proc()
                .process_link(lref, link, &self.cache, path)
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            if !artifact.is_link_valid() {
                for record in &produced {
                    self.cache.remove_artifact(lref, record.proc_id());
                }
                return Ok(StageRun {
                    outcome: LinkOutcome::Rejected { proc_id },
                    produced: Vec::new(),
                });
            }

            let encoded = artifact
                .to_buf_dyn()
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            let data = ProcessorArtifactData::new(stage.chain_proc().proc_version(), encoded);
            produced.push(ArtifactRecord::new(lref.clone(), proc_id, data));
            self.cache.insert_artifact(lref.clone(), proc_id, artifact);
            self.outdated.remove_artifact(lref, proc_id);
        }
        Ok(StageRun {
            outcome: LinkOutcome::Accepted,
            produced,
        })
    }

    /// Lets each stage clean up after its artifact for a link being
    /// forgotten, then drops the cached artifacts.
    pub(crate) fn discard(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            if let Some(artifact) = self.cache.get_artifact_dyn(lref, proc_id) {
                stage
                    .chain_proc()
                    .preprune_artifact(lref, artifact.as_ref())
                    .map_err(|e| GExecError::Proc(proc_id, e))?;
            }
        }

        self.cache.remove_link(lref);
        self.outdated.remove_link(lref);
        Ok(())
    }

    /// Asks a stage whether it can pick up processing from a node.
    pub(crate) fn check_can_resume_at(
        &self,
        proc_id: ProcId,
        node: &NodeRef<S>,
    ) -> Result<bool, GExecError> {
        self.get_proc_stage(proc_id)
            .chain_proc()
            .check_can_resume_at(node)
            .map_err(|e| GExecError::Proc(proc_id, e))
    }

    /// Initializes a stage's aggregated state at a node.
    pub(crate) fn init_stage(&self, proc_id: ProcId, node: &NodeRef<S>) -> Result<(), GExecError> {
        self.get_proc_stage(proc_id)
            .chain_proc()
            .on_init(node)
            .map_err(|e| GExecError::Proc(proc_id, e))
    }

    /// Commits a path in one stage, which must have cached artifacts for
    /// every link on it.
    pub(crate) fn commit_stage(
        &self,
        proc_id: ProcId,
        path: &LinkPath<S>,
    ) -> Result<(), GExecError> {
        let artifacts = self.cached_artifacts(proc_id, path.links())?;
        self.get_proc_stage(proc_id)
            .chain_proc()
            .commit_outputs(path, &artifacts)
            .map_err(|e| GExecError::Proc(proc_id, e))
    }

    /// Checks that every stage could undo a committed path, which needs the
    /// artifact each committed every link on it with.  A stage added after a
    /// link was committed has none for it.
    pub(crate) fn check_undoable(&self, path: &LinkPath<S>) -> Result<(), GExecError> {
        for proc_id in self.pipeline.proc_ids() {
            self.committed_artifacts(proc_id, path.links())?;
        }
        Ok(())
    }

    /// Undoes a committed path in one stage, with the artifacts that
    /// committed it even if they no longer stand for their links.
    pub(crate) fn uncommit_stage(
        &self,
        proc_id: ProcId,
        path: &LinkPath<S>,
    ) -> Result<(), GExecError> {
        let artifacts = self.committed_artifacts(proc_id, path.links())?;
        self.get_proc_stage(proc_id)
            .chain_proc()
            .uncommit_outputs(path, &artifacts)
            .map_err(|e| GExecError::Proc(proc_id, e))
    }

    /// Lets every stage discard what it kept for rolling back to before a
    /// node.
    pub(crate) fn prune_upto(&self, node: &NodeRef<S>) -> Result<(), GExecError> {
        for stage in self.pipeline.iter_stages() {
            stage
                .chain_proc()
                .prune_state_upto(node)
                .map_err(|e| GExecError::Proc(stage.proc_id(), e))?;
        }
        Ok(())
    }

    fn get_proc_stage(&self, proc_id: ProcId) -> &Stage<S> {
        self.pipeline
            .get_stage(proc_id)
            .expect("gchain: proc ID doesn't point to stage")
    }

    /// Collects a stage's cached artifacts for a run of links, in order.
    fn cached_artifacts(
        &self,
        proc_id: ProcId,
        links: &[LinkRef<S>],
    ) -> Result<Vec<Arc<dyn DynProcArtifact>>, GExecError> {
        links
            .iter()
            .map(|lref| {
                self.cache
                    .get_artifact_dyn(lref, proc_id)
                    .cloned()
                    .ok_or_else(|| GExecError::missing_artifact(lref, proc_id))
            })
            .collect()
    }

    /// Collects the artifacts a stage committed a run of links with, in
    /// order, which are the outdated ones where there are any.
    fn committed_artifacts(
        &self,
        proc_id: ProcId,
        links: &[LinkRef<S>],
    ) -> Result<Vec<Arc<dyn DynProcArtifact>>, GExecError> {
        links
            .iter()
            .map(|lref| {
                self.outdated
                    .get_artifact_dyn(lref, proc_id)
                    .or_else(|| self.cache.get_artifact_dyn(lref, proc_id))
                    .cloned()
                    .ok_or_else(|| GExecError::stale_artifact(lref, proc_id))
            })
            .collect()
    }

    /// Works out which stages have covered a link and the path to its origin
    /// from what's in the cache.
    fn link_coverage(&self, lref: &LinkRef<S>, path: &LinkPath<S>) -> LinkCoverage {
        let mut coverage = LinkCoverage::new();
        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            let accepted_cur = self
                .cache
                .get_artifact_dyn(lref, proc_id)
                .is_some_and(|a| a.is_link_valid());
            if accepted_cur {
                coverage.mark_cur(proc_id);
            }

            let covers_path = path
                .links()
                .iter()
                .all(|l| self.cache.get_artifact_dyn(l, proc_id).is_some());
            if covers_path {
                coverage.mark_path(proc_id);
            }
        }
        coverage
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::test_support::*;

    fn id(s: &str) -> ProcId {
        ProcId::from_str(s).expect("test: parse ProcId")
    }

    fn ids(names: &[&str]) -> Vec<ProcId> {
        names.iter().map(|n| id(n)).collect()
    }

    /// A path from a base node through links given as `(lref, target)`.
    fn path(base: u8, steps: &[(u8, u8)]) -> LinkPath<TestSpec> {
        LinkPath::from_steps(
            TestRef(base),
            steps.iter().map(|(l, n)| (TestRef(*l), TestRef(*n))),
        )
    }

    fn record(lref: u8, proc: &str, version: u32) -> ArtifactRecord<TestSpec> {
        let data =
            ProcessorArtifactData::from_artifact(ProcVersion::from(version), &FlagArtifact(true))
                .expect("test: encode artifact");
        ArtifactRecord::new(TestRef(lref), id(proc), data)
    }

    fn runner(procs: Vec<TestProc>) -> StageRunner<TestSpec> {
        StageRunner::new(pipeline_of(procs))
    }

    #[test]
    fn test_stored_artifacts_count_only_at_the_current_version() {
        let mut runner = runner(vec![TestProc::new(), TestProc::new()]);
        assert_eq!(runner.missing_stages(&TestRef(10)), ids(&["a", "b"]));

        for rec in [record(10, "a", 1), record(12, "gone", 1)] {
            runner
                .insert_stored(rec, None)
                .expect("test: insert stored");
        }

        assert_eq!(runner.missing_stages(&TestRef(10)), ids(&["b"]));
        assert_eq!(runner.missing_stages(&TestRef(12)), ids(&["a", "b"]));

        let err = runner.insert_stored(record(11, "a", 2), None).unwrap_err();
        assert!(matches!(err, GExecError::ArtifactFromNewerVersion { .. }));
    }

    #[test]
    fn test_older_artifacts_count_where_the_stage_accepts_them() {
        let proc = TestProc::new().with_version(2).accepting_old([10]);
        let mut runner = runner(vec![proc]);

        for lref in [10, 11] {
            assert!(runner.check_needs_header(&record(lref, "a", 1)));
            runner
                .insert_stored(record(lref, "a", 1), Some(&TestLink(lref)))
                .expect("test: insert stored");
        }
        assert!(!runner.check_needs_header(&record(12, "a", 2)));

        assert!(runner.missing_stages(&TestRef(10)).is_empty());
        assert_eq!(runner.missing_stages(&TestRef(11)), ids(&["a"]));
    }

    #[test]
    fn test_rejection_stops_later_stages_and_drops_what_the_run_made() {
        let first = TestProc::new();
        let second = TestProc::new().rejecting([10]);
        let third = TestProc::new();
        let third_events = third.events();
        let mut runner = runner(vec![first, second, third]);

        let run = runner
            .run(
                &TestRef(10),
                &TestLink(10),
                &path(1, &[]),
                &ids(&["a", "b", "c"]),
            )
            .expect("test: run stages");

        assert_eq!(run.outcome(), LinkOutcome::Rejected { proc_id: id("b") });
        assert!(run.produced().is_empty());
        assert!(take_events(&third_events).is_empty());
        assert!(
            runner
                .get_artifact::<FlagArtifact>(&TestRef(10), id("a"))
                .is_none()
        );
    }

    #[test]
    fn test_run_covers_only_the_stages_asked_for() {
        let first = TestProc::new();
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let mut runner = runner(vec![first, second]);

        let run = runner
            .run(&TestRef(10), &TestLink(10), &path(1, &[]), &ids(&["b"]))
            .expect("test: run stages");

        assert_eq!(run.outcome(), LinkOutcome::Accepted);
        assert_eq!(run.produced().len(), 1);
        assert!(take_events(&first_events).is_empty());
        assert_eq!(
            take_events(&second_events),
            vec![ProcEvent::Process(TestRef(10))]
        );
    }

    /// A stage can undo a link with the artifact it committed it with, even
    /// one it wouldn't stand behind any more, but not with nothing.
    #[test]
    fn test_undo_takes_the_artifact_each_stage_committed_with() {
        let first = TestProc::new().with_version(2);
        let first_events = first.events();
        let mut runner = runner(vec![first, TestProc::new()]);
        let path = path(1, &[(10, 2)]);
        runner
            .insert_stored(record(10, "a", 1), Some(&TestLink(10)))
            .expect("test: insert stored");

        assert_eq!(runner.missing_stages(&TestRef(10)), ids(&["a", "b"]));
        let err = runner.check_undoable(&path).unwrap_err();
        assert!(matches!(err, GExecError::StaleArtifact { proc_id, .. } if proc_id == id("b")));
        let err = runner.commit_stage(id("a"), &path).unwrap_err();
        assert!(matches!(err, GExecError::MissingArtifact { proc_id, .. } if proc_id == id("a")));

        runner
            .uncommit_stage(id("a"), &path)
            .expect("test: uncommit a");
        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Uncommit(vec![TestRef(10)])]
        );
    }

    #[test]
    fn test_discard_only_prepunes_cached_artifacts() {
        let first = TestProc::new();
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let mut runner = runner(vec![first, second]);
        runner
            .run(&TestRef(10), &TestLink(10), &path(1, &[]), &ids(&["a"]))
            .expect("test: run stages");
        take_events(&first_events);

        runner.discard(&TestRef(10)).expect("test: discard");

        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Preprune(TestRef(10))]
        );
        assert!(take_events(&second_events).is_empty());
        assert_eq!(runner.missing_stages(&TestRef(10)), ids(&["a", "b"]));
    }
}
