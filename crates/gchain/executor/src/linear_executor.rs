//! Simple single-threaded executor.
//!
//! Runs every stage on a link serially in canonical order, and keeps every
//! link it has processed until told to commit, discard, or prune it.  It's
//! meant as the reference for what an executor owes the stages and the sync
//! engine driving it, not as the fastest way to do it.
//!
//! The executor's picture of the graph is node-centric: the stages' aggregated
//! states sit at the committed node, and a processed link's artifacts describe
//! the transition to its target node regardless of how that node is otherwise
//! reached.  So a link may be processed whenever its origin is reachable from
//! the committed node through processed links, and the path handed to the
//! stages is whichever such path has the fewest links.
//!
//! The executor keeps no picture of the graph itself.  The provider is asked
//! about the graph whenever a path is needed, and a link counts as processed
//! iff its artifacts are in the store, which is also where they live between
//! uses: only the committed path's artifacts are kept in memory (a commit
//! can't be undone without them), and anything else is loaded when a path
//! runs through it.  The pipeline's position is a small [`TrackingState`]
//! mirrored whole in the store.  A [`StageRunner`] does the running and holds
//! the loaded artifacts; the executor is where every fetch and every store
//! write happens, in order.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::core::ExecutorCore;
use crate::errors::GExecError;

use crate::stage_runner::LinkOutcome;
use crate::store::ExecutorStore;

/// Linear processor pipeline executor.
///
/// This is still a "low initiative" data structure, it must be driven by some
/// external sync engine that decides which links to process and which paths to
/// commit.  The executor only reports when a request doesn't make sense
/// against what it has.
///
/// One only exists while every stage sits at the committed node and stands
/// behind the committed links since it was initialized.
pub struct LinearExecutor<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> {
    core: ExecutorCore<S, P, X>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>
    LinearExecutor<S, P, X>
{
    /*
        /// Opens an executor over a store, reconciling what it holds with the
        /// pipeline.
        ///
        /// A store that has never been used starts every stage at `base_node`,
        /// which is then the oldest node the pipeline can roll back to until it's
        /// pruned past.  Otherwise the committed path's artifacts are loaded.  If
        /// the stages accept them all, stages that have no committed state or lag
        /// the committed node are brought level and the executor is ready.  If
        /// not, what's handed back is the means to reprocess the links in
        /// question, which gives the executor once that's done.
        ///
        /// Uncommitted links are left where they are; one whose artifacts turn
        /// out to be missing or no longer accepted has those stages re-run when
        /// it's next processed.
        pub fn open(
            pipeline: StagePipeline<S>,
            provider: Arc<P>,
            store: Arc<X>,
            base_node: NodeRef<S>,
        ) -> Result<Opened<S, P, X>, GExecError> {
    }
        */

    /// Wraps a core opened on a clean database.
    ///
    /// This means all processor stage data is "level" with the committed node
    /// and stale artifacts have been reprocessed.
    pub(crate) fn from_clean_core(core: ExecutorCore<S, P, X>) -> Self {
        debug_assert!(core.check_clean(), "gchain/exec: core state not clean");
        Self { core }
    }

    /// The node every stage has committed up to.
    pub fn committed_node(&self) -> &NodeRef<S> {
        self.core.committed_node()
    }

    /// The committed links that can still be rolled back, from the oldest.
    pub fn committed_path(&self) -> &LinkPath<S> {
        self.core.committed_path()
    }

    /// The artifact a stage produced for a processed link.
    pub fn get_artifact<A: ProcArtifact>(
        &mut self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Result<Option<Arc<A>>, GExecError> {
        self.core.get_artifact(lref, proc_id)
    }

    /// Runs every stage on a link whose origin is reachable from the committed
    /// node, recording it if they all accept it.
    ///
    /// Processing a link every stage has already accepted is a no-op
    /// reporting acceptance; one some stages haven't (never run, or run by
    /// a version of the stage it no longer accepts) has just those stages
    /// run.  A rejected link leaves nothing behind, so it can be asked about
    /// again.
    pub fn process_link(&mut self, lref: &LinkRef<S>) -> Result<LinkOutcome, GExecError> {
        let missing = self.core.get_missing_stages(lref)?;
        if missing.is_empty() {
            return Ok(LinkOutcome::Accepted);
        }

        let endpoints = self.core.fetch_link_endpoints(lref)?;
        let path = self.core.path_to_origin(lref, endpoints.origin())?;

        let outcome = self.core.run_stages(lref, &path, &missing)?;
        if matches!(outcome, LinkOutcome::Rejected { .. }) {
            // Whatever an earlier run had stored for it goes too, along with
            // anything that was built on it.
            self.core.forget_link(lref)?;
            self.core.sweep_from(endpoints.target())?;
        }
        Ok(outcome)
    }

    /// Commits the path from the committed node through a processed link, in
    /// every stage.
    ///
    /// The path is the one with the fewest links to the link's origin.  The
    /// links stay recorded afterwards so the commit can be undone.
    pub fn commit_through(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        if self.core.tracking().is_committed(lref) {
            return Err(GExecError::link_on_committed_path(lref));
        }
        if !self.core.check_usable(lref)? {
            return Err(GExecError::link_not_processed(lref));
        }

        let endpoints = self.core.fetch_link_endpoints(lref)?;
        let mut path = self.core.path_to_origin(lref, endpoints.origin())?;
        let pushed = path.try_push_link(lref.clone(), &endpoints);
        debug_assert!(pushed, "gchain: path found to the link's own origin");

        self.core.commit_path(&path)
    }

    /// Rolls every stage back to a node on the committed path, undoing the
    /// links after it in reverse canonical order, a commit at a time from the
    /// newest.
    ///
    /// The undone links stay recorded as uncommitted, so they can be committed
    /// again or built on.
    pub fn uncommit_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        let undone = self.core.check_undoable_from(node)?;
        if undone.is_empty() {
            return Ok(());
        }

        let committed = self.committed_node().clone();
        for proc_id in self.core.get_proc_ids().into_iter().rev() {
            self.core
                .uncommit_stage_between(proc_id, node, &committed)?;
        }
        self.core.truncate_committed_to(node)
    }

    /// Forgets an uncommitted link along with every link that was only
    /// reachable through it, returning everything forgotten.
    pub fn discard_link(&mut self, lref: &LinkRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        if self.core.tracking().is_committed(lref) {
            return Err(GExecError::link_on_committed_path(lref));
        }
        if !self.core.check_present(lref)? {
            return Err(GExecError::link_not_processed(lref));
        }

        let endpoints = self.core.fetch_link_endpoints(lref)?;
        self.core.forget_link(lref)?;
        let mut dropped = vec![lref.clone()];
        dropped.extend(self.core.sweep_from(endpoints.target())?);
        Ok(dropped)
    }

    /// Gives up the ability to roll back to before a node on the committed
    /// path, forgetting the links before it and everything hanging off them,
    /// and lets the stages discard what they kept for that.
    ///
    /// Returns every link forgotten.
    pub fn prune_upto(&mut self, node: &NodeRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        let dropped = self.core.advance_committed_base_to(node)?;
        self.core.prune_stages_upto(node)?;
        Ok(dropped)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::config::{ExecutorBuilder, PipelineBuilder, StagePipeline};
    use crate::mem_store::MemExecutorStore;
    use crate::open::{OpenReport, OpenResult};
    use crate::reprocess::ReprocExecutor;
    use crate::test_support::*;
    use crate::tracking::{CommitIndex, TrackingState};

    type Exec = LinearExecutor<TestSpec, TestProvider, MemExecutorStore<TestSpec>>;
    type Reproc = ReprocExecutor<TestSpec, TestProvider, MemExecutorStore<TestSpec>>;

    fn id(s: &str) -> ProcId {
        ProcId::from_str(s).expect("test: parse ProcId")
    }

    fn refs(ids: &[u8]) -> Vec<TestRef> {
        ids.iter().copied().map(TestRef).collect()
    }

    /// Blocks 1→2→3→4 as links 10, 11, 12, a checkpoint 1→3 as link 20, and
    /// a side branch 2→9 as link 30.
    fn provider() -> Arc<TestProvider> {
        let p = TestProvider::new();
        p.add_link(10, 1, 2);
        p.add_link(11, 2, 3);
        p.add_link(12, 3, 4);
        p.add_link(20, 1, 3);
        p.add_link(30, 2, 9);
        Arc::new(p)
    }

    /// Opens an executor at genesis node 1 over a pipeline.
    fn open_pipeline(
        provider: &Arc<TestProvider>,
        store: &Arc<MemExecutorStore<TestSpec>>,
        pipeline: StagePipeline<TestSpec>,
    ) -> OpenResult<TestSpec, TestProvider, MemExecutorStore<TestSpec>> {
        ExecutorBuilder::new(
            pipeline,
            Arc::clone(provider),
            Arc::clone(store),
            TestRef(1),
        )
        .open()
        .expect("test: open executor")
    }

    /// Opens an executor at genesis node 1 with the given stages under the
    /// IDs "a", "b", ... in order, which has to need no reprocessing.
    fn open(
        provider: &Arc<TestProvider>,
        store: &Arc<MemExecutorStore<TestSpec>>,
        procs: Vec<TestProc>,
    ) -> (Exec, OpenReport<TestSpec>) {
        open_pipeline(provider, store, pipeline_of(procs)).expect_ready()
    }

    /// Opens an executor like [`open`], which has to need reprocessing.
    fn open_reproc(
        provider: &Arc<TestProvider>,
        store: &Arc<MemExecutorStore<TestSpec>>,
        pipeline: StagePipeline<TestSpec>,
    ) -> Reproc {
        match open_pipeline(provider, store, pipeline) {
            OpenResult::Ready(..) => panic!("test: expected links to reprocess"),
            OpenResult::NeedsReprocess(pending) => pending,
        }
    }

    /// A store with link 10 committed on its own and then 11 and 12
    /// together, by version 1 of stages "a" and "b", which don't depend on
    /// each other.  Link 30 is processed but not committed.
    fn committed_store() -> (Arc<TestProvider>, Arc<MemExecutorStore<TestSpec>>) {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        let (mut exec, _) = open(&provider, &store, vec![TestProc::new(), TestProc::new()]);
        for lref in [10, 11, 12, 30] {
            accept(&mut exec, lref);
        }
        exec.commit_through(&TestRef(10)).expect("test: commit");
        exec.commit_through(&TestRef(12)).expect("test: commit");
        (provider, store)
    }

    /// The links of each plan that's left, by stage.
    fn planned(pending: &Reproc) -> Vec<(ProcId, TestRef, Vec<TestRef>)> {
        pending
            .plans()
            .map(|p| {
                (
                    p.proc_id(),
                    *p.path().base_node(),
                    p.path().links().to_vec(),
                )
            })
            .collect()
    }

    fn fresh(procs: Vec<TestProc>) -> (Exec, Arc<TestProvider>, Arc<MemExecutorStore<TestSpec>>) {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        let (exec, _) = open(&provider, &store, procs);
        (exec, provider, store)
    }

    fn accept(exec: &mut Exec, lref: u8) {
        let outcome = exec
            .process_link(&TestRef(lref))
            .expect("test: process link");
        assert_eq!(outcome, LinkOutcome::Accepted, "test: link {lref}");
    }

    /// Whether stage "a" has a current artifact for a link, which is how the
    /// tests tell a processed link from one that isn't.
    fn has_artifact(exec: &mut Exec, lref: u8) -> bool {
        exec.get_artifact::<FlagArtifact>(&TestRef(lref), id("a"))
            .expect("test: fetch artifact")
            .is_some()
    }

    fn tracking(store: &MemExecutorStore<TestSpec>) -> TrackingState<TestSpec> {
        store
            .load_tracking()
            .expect("test: load tracking")
            .expect("test: tracking stored")
    }

    #[test]
    fn test_fresh_open_inits_every_stage_at_genesis() {
        let proc = TestProc::new();
        let events = proc.events();
        let (exec, _, store) = fresh(vec![proc]);

        assert_eq!(take_events(&events), vec![ProcEvent::Init(TestRef(1))]);
        assert_eq!(exec.committed_node(), &TestRef(1));
        assert!(exec.committed_path().is_empty());
        let stored = tracking(&store);
        assert!(stored.committed_path().is_empty());
        assert_eq!(stored.committed_node(), &TestRef(1));
        assert_eq!(stored.get_stage_node(id("a")), Some(&TestRef(1)));
    }

    #[test]
    fn test_process_and_commit_round_trip() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, store) = fresh(vec![proc]);
        take_events(&events);

        accept(&mut exec, 10);
        accept(&mut exec, 11);
        // Processing a recorded link again is a no-op.
        accept(&mut exec, 11);
        assert!(has_artifact(&mut exec, 11));
        assert!(exec.committed_path().is_empty());

        exec.commit_through(&TestRef(11)).expect("test: commit");

        assert_eq!(
            take_events(&events),
            vec![
                ProcEvent::Process(TestRef(10)),
                ProcEvent::Process(TestRef(11)),
                ProcEvent::Commit(refs(&[10, 11])),
            ]
        );
        assert_eq!(exec.committed_node(), &TestRef(3));
        assert_eq!(exec.committed_path().links(), &refs(&[10, 11]));
        let stored = tracking(&store);
        assert_eq!(stored.get_stage_node(id("a")), Some(&TestRef(3)));
        assert_eq!(stored.committed_path().links(), &refs(&[10, 11]));
        // Committed links keep their artifacts so the commit can be undone.
        assert!(has_artifact(&mut exec, 10));
    }

    #[test]
    fn test_rejection_stops_later_stages_and_leaves_no_record() {
        let first = TestProc::new().rejecting([10]);
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let (mut exec, _, store) = fresh(vec![first, second]);
        take_events(&first_events);
        take_events(&second_events);

        let outcome = exec.process_link(&TestRef(10)).expect("test: process link");

        assert_eq!(outcome, LinkOutcome::Rejected { proc_id: id("a") });
        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Process(TestRef(10))]
        );
        assert!(take_events(&second_events).is_empty());
        assert!(!has_artifact(&mut exec, 10));
        assert!(!has_stored(&store, 10));
    }

    #[test]
    fn test_link_from_unreachable_origin_is_refused() {
        let (mut exec, _, _) = fresh(vec![TestProc::new()]);

        let err = exec.process_link(&TestRef(12)).unwrap_err();
        assert!(matches!(err, GExecError::OriginUnreachable(_)));

        // After the path to its origin exists it's fine.
        accept(&mut exec, 20);
        accept(&mut exec, 12);
    }

    /// The checkpoint and the two blocks both reach node 3, and the commit
    /// path takes whichever has fewer links.
    #[test]
    fn test_commit_takes_path_with_fewest_links() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, _) = fresh(vec![proc]);
        for lref in [10, 11, 20, 12] {
            accept(&mut exec, lref);
        }
        take_events(&events);

        exec.commit_through(&TestRef(12)).expect("test: commit");

        assert_eq!(
            take_events(&events),
            vec![ProcEvent::Commit(refs(&[20, 12]))]
        );
        assert_eq!(exec.committed_path().links(), &refs(&[20, 12]));
        // The block links are still around, just not on the committed path.
        assert!(has_artifact(&mut exec, 11));
    }

    #[test]
    fn test_commit_refuses_unprocessed_or_committed_links() {
        let (mut exec, _, _) = fresh(vec![TestProc::new()]);
        accept(&mut exec, 10);

        let err = exec.commit_through(&TestRef(11)).unwrap_err();
        assert!(matches!(err, GExecError::LinkNotProcessed(_)));

        exec.commit_through(&TestRef(10)).expect("test: commit");
        let err = exec.commit_through(&TestRef(10)).unwrap_err();
        assert!(matches!(err, GExecError::LinkOnCommittedPath(_)));
    }

    #[test]
    fn test_uncommit_walks_committed_path_back_in_reverse_stage_order() {
        let first = TestProc::new();
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let (mut exec, _, store) = fresh(vec![first, second]);
        accept(&mut exec, 10);
        accept(&mut exec, 11);
        exec.commit_through(&TestRef(11)).expect("test: commit");
        take_events(&first_events);
        take_events(&second_events);

        exec.uncommit_to(&TestRef(2)).expect("test: uncommit");

        assert_eq!(
            take_events(&second_events),
            vec![ProcEvent::Uncommit(refs(&[11]))]
        );
        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Uncommit(refs(&[11]))]
        );
        assert_eq!(exec.committed_node(), &TestRef(2));
        assert_eq!(exec.committed_path().links(), &refs(&[10]));
        assert_eq!(tracking(&store).get_stage_node(id("b")), Some(&TestRef(2)));

        // The undone link is uncommitted again, so it can be built on and
        // committed again.
        accept(&mut exec, 30);
        exec.commit_through(&TestRef(11)).expect("test: recommit");
        assert_eq!(exec.committed_node(), &TestRef(3));
        take_events(&first_events);

        // Uncommitting to where we already are does nothing.
        exec.uncommit_to(&TestRef(3)).expect("test: uncommit noop");
        assert!(take_events(&first_events).is_empty());

        let err = exec.uncommit_to(&TestRef(9)).unwrap_err();
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));
    }

    #[test]
    fn test_discard_link_drops_everything_only_reachable_through_it() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, store) = fresh(vec![proc]);
        for lref in [10, 11, 30, 20] {
            accept(&mut exec, lref);
        }
        take_events(&events);

        let mut dropped = exec.discard_link(&TestRef(10)).expect("test: discard");
        dropped.sort();

        // Node 3 is still reached by the checkpoint, so 11's target survives
        // but 11 itself and the branch off node 2 don't.
        assert_eq!(dropped, refs(&[10, 11, 30]));
        let mut prepruned: Vec<_> = take_events(&events)
            .into_iter()
            .map(|e| match e {
                ProcEvent::Preprune(l) => l,
                other => panic!("test: unexpected event {other:?}"),
            })
            .collect();
        prepruned.sort();
        assert_eq!(prepruned, refs(&[10, 11, 30]));
        assert!(has_artifact(&mut exec, 20));
        assert!(!has_stored(&store, 11));
        assert!(has_stored(&store, 20));

        let err = exec.discard_link(&TestRef(10)).unwrap_err();
        assert!(matches!(err, GExecError::LinkNotProcessed(_)));

        exec.commit_through(&TestRef(20)).expect("test: commit");
        let err = exec.discard_link(&TestRef(20)).unwrap_err();
        assert!(matches!(err, GExecError::LinkOnCommittedPath(_)));
    }

    #[test]
    fn test_prune_forgets_pruned_links_and_branches_off_them() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, store) = fresh(vec![proc]);
        for lref in [10, 11, 30, 12] {
            accept(&mut exec, lref);
        }
        exec.commit_through(&TestRef(11)).expect("test: commit");
        take_events(&events);

        let mut dropped = exec.prune_upto(&TestRef(3)).expect("test: prune");
        dropped.sort();

        assert_eq!(dropped, refs(&[10, 11, 30]));
        assert_eq!(
            take_events(&events).last(),
            Some(&ProcEvent::Prune(TestRef(3)))
        );
        assert_eq!(exec.committed_node(), &TestRef(3));
        assert_eq!(exec.committed_path().base_node(), &TestRef(3));
        assert!(exec.committed_path().is_empty());
        assert!(has_artifact(&mut exec, 12));
        assert!(!has_stored(&store, 10));
        let stored = tracking(&store);
        assert_eq!(stored.committed_path().base_node(), &TestRef(3));
        assert!(stored.committed_path().is_empty());

        // Nothing before the new base can be rolled back to any more.
        let err = exec.uncommit_to(&TestRef(1)).unwrap_err();
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));
    }

    #[test]
    fn test_reopen_restores_committed_path_and_processed_links() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            accept(&mut exec, 10);
            accept(&mut exec, 11);
            accept(&mut exec, 30);
            exec.commit_through(&TestRef(10)).expect("test: commit");
        }

        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, report) = open(&provider, &store, vec![proc]);

        assert!(take_events(&events).is_empty());
        assert!(report.initialized().is_empty());
        assert!(report.recommitted().is_empty());
        assert!(report.reprocessed().is_empty());
        assert_eq!(exec.committed_node(), &TestRef(2));
        assert_eq!(exec.committed_path().links(), &refs(&[10]));
        assert!(has_artifact(&mut exec, 11));
        assert!(has_artifact(&mut exec, 30));

        accept(&mut exec, 12);
        exec.commit_through(&TestRef(12)).expect("test: commit");
        assert_eq!(
            take_events(&events),
            vec![
                ProcEvent::Process(TestRef(12)),
                ProcEvent::Commit(refs(&[11, 12])),
            ]
        );
    }

    #[test]
    fn test_reopen_inits_new_stage_and_recommits_lagging_stage() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            accept(&mut exec, 10);
            accept(&mut exec, 11);
            exec.commit_through(&TestRef(11)).expect("test: commit");
        }
        // Pretend stage "a" crashed after committing only the first link.
        let mut lagging = tracking(&store);
        lagging.set_stage_node(id("a"), TestRef(2));
        store
            .store_tracking(&lagging)
            .expect("test: store tracking");

        let first = TestProc::new();
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let (exec, report) = open(&provider, &store, vec![first, second]);

        assert_eq!(report.recommitted(), &[id("a")]);
        assert_eq!(report.initialized(), &[id("b")]);
        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Commit(refs(&[11]))]
        );
        assert_eq!(
            take_events(&second_events),
            vec![ProcEvent::Init(TestRef(3))]
        );
        assert_eq!(exec.committed_node(), &TestRef(3));
        let stored = tracking(&store);
        assert_eq!(stored.get_stage_node(id("a")), Some(&TestRef(3)));
        assert_eq!(stored.get_stage_node(id("b")), Some(&TestRef(3)));
    }

    #[test]
    fn test_stale_and_missing_stages_rerun_when_link_is_next_processed() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            accept(&mut exec, 10);
            accept(&mut exec, 11);
        }

        // Stage "a" changed behavior, and stage "b" is new.
        let first = TestProc::new().with_version(2);
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let (mut exec, report) = open(&provider, &store, vec![first, second]);
        assert_eq!(report.initialized(), &[id("b")]);
        assert!(take_events(&first_events).is_empty());
        assert_eq!(
            take_events(&second_events),
            vec![ProcEvent::Init(TestRef(1))]
        );

        accept(&mut exec, 10);
        accept(&mut exec, 11);

        assert_eq!(
            take_events(&first_events),
            vec![
                ProcEvent::Process(TestRef(10)),
                ProcEvent::Process(TestRef(11)),
            ]
        );
        assert_eq!(
            take_events(&second_events),
            vec![
                ProcEvent::Process(TestRef(10)),
                ProcEvent::Process(TestRef(11)),
            ]
        );
        assert_eq!(
            stored_artifact(&store, TestRef(10), id("a")).map(|d| d.exec_version()),
            Some(ProcVersion::from(2))
        );
    }

    /// A link the new version of a stage rejects is dropped, and so is
    /// everything that was built on it.
    #[test]
    fn test_rerun_rejection_discards_link_and_what_was_built_on_it() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            for lref in [10, 11, 30, 20] {
                accept(&mut exec, lref);
            }
        }

        let proc = TestProc::new().with_version(2).rejecting([10]);
        let (mut exec, _) = open(&provider, &store, vec![proc]);

        let outcome = exec.process_link(&TestRef(10)).expect("test: process link");

        assert_eq!(outcome, LinkOutcome::Rejected { proc_id: id("a") });
        assert!(!has_stored(&store, 10));
        assert!(!has_stored(&store, 11));
        assert!(!has_stored(&store, 30));
        // The checkpoint reaches node 3 on its own, so it survives with its
        // old artifacts until it's processed again.
        assert!(has_stored(&store, 20));
        assert!(!has_artifact(&mut exec, 20));
        accept(&mut exec, 20);
        assert!(has_artifact(&mut exec, 20));
    }

    /// A stage added after links were committed has nothing for them, so it
    /// starts from the committed node and they can't be undone.
    #[test]
    fn test_links_committed_before_a_stage_was_added_cant_be_undone() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            accept(&mut exec, 10);
            accept(&mut exec, 11);
            exec.commit_through(&TestRef(10)).expect("test: commit");
        }

        let (first, second) = (TestProc::new(), TestProc::new());
        let (first_events, second_events) = (first.events(), second.events());
        let (mut exec, report) = open(&provider, &store, vec![first, second]);

        assert_eq!(report.initialized(), &[id("b")]);
        assert_eq!(
            take_events(&second_events),
            vec![ProcEvent::Init(TestRef(2))]
        );
        accept(&mut exec, 11);
        exec.commit_through(&TestRef(11)).expect("test: commit");

        let err = exec.uncommit_to(&TestRef(1)).unwrap_err();
        assert!(matches!(err, GExecError::StaleArtifact { proc_id, .. } if proc_id == id("b")));
        take_events(&first_events);
        exec.uncommit_to(&TestRef(2)).expect("test: uncommit");
        assert_eq!(
            take_events(&first_events),
            vec![ProcEvent::Uncommit(refs(&[11]))]
        );

        // Reopening doesn't have the stage go back over what it was never
        // there for.
        drop(exec);
        open(&provider, &store, vec![TestProc::new(), TestProc::new()]);
    }

    /// Only the committed path stays loaded across a commit; anything else
    /// comes back from the store when a path needs it.
    #[test]
    fn test_commit_evicts_uncommitted_artifacts_but_they_reload() {
        let (mut exec, _, _) = fresh(vec![TestProc::new()]);
        accept(&mut exec, 10);
        accept(&mut exec, 30);
        exec.commit_through(&TestRef(10)).expect("test: commit");

        assert!(exec.core.check_loaded(&TestRef(10)));
        assert!(!exec.core.check_loaded(&TestRef(30)));
        assert!(has_artifact(&mut exec, 30));
        assert!(exec.core.check_loaded(&TestRef(30)));
    }

    #[test]
    fn test_commit_log_follows_the_committed_path() {
        let (provider, store) = committed_store();
        let (first, second) = (TestProc::new(), TestProc::new());
        let events = first.events();
        let (mut exec, _) = open(&provider, &store, vec![first, second]);

        assert_eq!(stored_segment(&store, 0), Some(refs(&[10])));
        assert_eq!(stored_segment(&store, 1), Some(refs(&[11, 12])));
        assert_eq!(stored_segment(&store, 2), None);

        // Rolling back into a commit leaves the part of it that stands.
        exec.uncommit_to(&TestRef(3)).expect("test: uncommit");
        assert_eq!(stored_segment(&store, 1), Some(refs(&[11])));
        exec.commit_through(&TestRef(12)).expect("test: commit");
        assert_eq!(stored_segment(&store, 2), Some(refs(&[12])));

        // Rolling back across commits undoes them one at a time.
        take_events(&events);
        exec.uncommit_to(&TestRef(2)).expect("test: uncommit");
        assert_eq!(
            take_events(&events),
            vec![
                ProcEvent::Uncommit(refs(&[12])),
                ProcEvent::Uncommit(refs(&[11])),
            ]
        );
        assert_eq!(stored_segment(&store, 0), Some(refs(&[10])));
        assert_eq!(stored_segment(&store, 1), None);
        assert_eq!(stored_segment(&store, 2), None);

        exec.commit_through(&TestRef(12)).expect("test: commit");
        exec.prune_upto(&TestRef(3)).expect("test: prune");
        assert_eq!(stored_segment(&store, 0), None);
        assert_eq!(stored_segment(&store, 1), Some(refs(&[12])));
        let stored = tracking(&store);
        assert_eq!(stored.first_commit(), CommitIndex::from(1));
        assert_eq!(stored.last_commit(), Some(CommitIndex::from(1)));
    }

    #[test]
    fn test_accepted_older_artifacts_need_no_reprocessing() {
        let (provider, store) = committed_store();

        let first = TestProc::new()
            .with_version(2)
            .accepting_old([10, 11, 12, 30]);
        let events = first.events();
        let (mut exec, report) = open(&provider, &store, vec![first, TestProc::new()]);

        assert!(report.reprocessed().is_empty());
        assert!(has_artifact(&mut exec, 12));
        accept(&mut exec, 30);
        exec.uncommit_to(&TestRef(2)).expect("test: uncommit");
        assert_eq!(
            take_events(&events),
            vec![ProcEvent::Uncommit(refs(&[11, 12]))]
        );
    }

    #[test]
    fn test_unaccepted_committed_links_are_reprocessed_by_their_stage() {
        let (provider, store) = committed_store();

        let first = TestProc::new().with_version(2).accepting_old([10]);
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let mut pending = open_reproc(&provider, &store, pipeline_of(vec![first, second]));

        assert_eq!(
            planned(&pending),
            vec![(id("a"), TestRef(2), refs(&[11, 12]))]
        );
        assert_eq!(pending.committed_node(), &TestRef(4));
        assert!(take_events(&first_events).is_empty());

        let applied = pending.apply_next_stage().expect("test: apply stage");
        assert_eq!(applied, Some(id("a")));
        let (mut exec, report) = pending.finish().expect("test: finish");

        assert_eq!(
            take_events(&first_events),
            vec![
                ProcEvent::Uncommit(refs(&[11, 12])),
                ProcEvent::Process(TestRef(11)),
                ProcEvent::Process(TestRef(12)),
                ProcEvent::Commit(refs(&[11, 12])),
            ]
        );
        assert!(take_events(&second_events).is_empty());
        assert_eq!(report.reprocessed().len(), 1);
        assert!(report.rejected().is_empty());
        assert_eq!(exec.committed_path().links(), &refs(&[10, 11, 12]));

        let versions: Vec<_> = [10, 11, 12]
            .into_iter()
            .map(|l| stored_artifact(&store, TestRef(l), id("a")).map(|d| d.exec_version()))
            .collect();
        let (v1, v2) = (ProcVersion::from(1), ProcVersion::from(2));
        assert_eq!(versions, vec![Some(v1), Some(v2), Some(v2)]);

        // Uncommitted links are still only redone when they come up.
        exec.uncommit_to(&TestRef(2)).expect("test: uncommit");
        accept(&mut exec, 30);
        assert_eq!(
            take_events(&first_events),
            vec![
                ProcEvent::Uncommit(refs(&[11, 12])),
                ProcEvent::Process(TestRef(30)),
            ]
        );
    }

    /// A stage that can't resume from where its accepted artifacts end goes
    /// further back, a commit at a time.
    #[test]
    fn test_reprocessing_starts_from_a_node_the_stage_can_resume_at() {
        let (provider, store) = committed_store();

        let first = TestProc::new()
            .with_version(2)
            .accepting_old([10, 11])
            .not_resuming_at([3, 2]);
        let events = first.events();
        let pending = open_reproc(&provider, &store, pipeline_of(vec![first, TestProc::new()]));

        assert_eq!(
            planned(&pending),
            vec![(id("a"), TestRef(1), refs(&[10, 11, 12]))]
        );
        pending.finish().expect("test: finish");

        assert_eq!(
            take_events(&events),
            vec![
                ProcEvent::Uncommit(refs(&[11, 12])),
                ProcEvent::Uncommit(refs(&[10])),
                ProcEvent::Process(TestRef(10)),
                ProcEvent::Process(TestRef(11)),
                ProcEvent::Process(TestRef(12)),
                ProcEvent::Commit(refs(&[10])),
                ProcEvent::Commit(refs(&[11, 12])),
            ]
        );
    }

    #[test]
    fn test_stage_with_nowhere_to_resume_fails_to_open() {
        let (provider, store) = committed_store();
        let first = TestProc::new().with_version(2).not_resuming_at([1, 2, 3]);

        let pipeline = pipeline_of(vec![first, TestProc::new()]);
        let res = ExecutorBuilder::new(pipeline, provider, store, TestRef(1)).open();

        let err = expect_err(res, "open to fail");
        assert!(matches!(err, GExecError::NoResumePoint(proc_id) if proc_id == id("a")));
    }

    /// What a stage built on another's artifacts is out of date once those
    /// are redone.
    #[test]
    fn test_dependent_stage_reprocesses_what_its_dep_does() {
        let (provider, store) = committed_store();

        let first = TestProc::new().with_version(2).accepting_old([10, 11]);
        let second = TestProc::new();
        let second_events = second.events();
        let no_deps = ProcDeps::new(Vec::new(), Vec::new());
        let on_first = ProcDeps::new(vec![id("a")], Vec::new());
        let pipeline = PipelineBuilder::new()
            .add_stage(id("a"), first, no_deps)
            .expect("test: add stage")
            .add_stage(id("b"), second, on_first)
            .expect("test: add stage")
            .build();
        let pending = open_reproc(&provider, &store, pipeline);

        assert_eq!(
            planned(&pending),
            vec![
                (id("a"), TestRef(3), refs(&[12])),
                (id("b"), TestRef(3), refs(&[12])),
            ]
        );
        pending.finish().expect("test: finish");

        assert_eq!(
            take_events(&second_events),
            vec![
                ProcEvent::Uncommit(refs(&[12])),
                ProcEvent::Process(TestRef(12)),
                ProcEvent::Commit(refs(&[12])),
            ]
        );
    }

    #[test]
    fn test_rejected_committed_link_rolls_the_pipeline_back_to_its_origin() {
        let (provider, store) = committed_store();

        let first = TestProc::new()
            .with_version(2)
            .accepting_old([10])
            .rejecting([12]);
        let second = TestProc::new();
        let (first_events, second_events) = (first.events(), second.events());
        let pending = open_reproc(&provider, &store, pipeline_of(vec![first, second]));

        let (mut exec, report) = pending.finish().expect("test: finish");

        assert_eq!(
            take_events(&first_events),
            vec![
                ProcEvent::Uncommit(refs(&[11, 12])),
                ProcEvent::Process(TestRef(11)),
                ProcEvent::Process(TestRef(12)),
                ProcEvent::Commit(refs(&[11])),
            ]
        );
        assert_eq!(
            take_events(&second_events),
            vec![
                ProcEvent::Uncommit(refs(&[12])),
                ProcEvent::Preprune(TestRef(12)),
            ]
        );

        let [rejected] = report.rejected() else {
            panic!("test: expected one rejection");
        };
        assert_eq!(rejected.lref(), &TestRef(12));
        assert_eq!(rejected.proc_id(), id("a"));
        assert_eq!(rejected.dropped(), &refs(&[12]));

        assert_eq!(exec.committed_path().links(), &refs(&[10, 11]));
        assert_eq!(stored_segment(&store, 1), Some(refs(&[11])));
        assert!(!has_stored(&store, 12));
        let stored = tracking(&store);
        assert_eq!(stored.get_stage_node(id("a")), Some(&TestRef(3)));
        assert_eq!(stored.get_stage_node(id("b")), Some(&TestRef(3)));

        let outcome = exec.process_link(&TestRef(12)).expect("test: process link");
        assert_eq!(outcome, LinkOutcome::Rejected { proc_id: id("a") });
    }
}
