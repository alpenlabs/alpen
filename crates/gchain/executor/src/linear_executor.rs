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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use strata_gchain_types::*;

use crate::config::StagePipeline;
use crate::errors::GExecError;
use crate::stage_runner::{LinkOutcome, StageRunner};
use crate::store::{ArtifactRecord, ExecutorStore};
use crate::tracking::TrackingState;
use crate::traverse::{find_path, find_reachable_links};

/// What [`LinearExecutor::open`] did to bring the stored state in line with
/// the pipeline it was opened with.
pub struct OpenReport<S: GChainSpec> {
    initialized: Vec<ProcId>,
    recommitted: Vec<ProcId>,
    stale_committed: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> OpenReport<S> {
    fn new() -> Self {
        Self {
            initialized: Vec::new(),
            recommitted: Vec::new(),
            stale_committed: Vec::new(),
        }
    }

    /// Stages that had no committed state and were initialized at the
    /// committed node.
    pub fn initialized(&self) -> &[ProcId] {
        &self.initialized
    }

    /// Stages whose committed node lagged the pipeline's and had the rest of
    /// the committed path committed again.
    pub fn recommitted(&self) -> &[ProcId] {
        &self.recommitted
    }

    /// Committed links some stage has no usable artifact for, because it was
    /// stored by another version of the stage or the stage was added after
    /// the link was committed.  They stay committed, but the path can't be
    /// rolled back across them.
    pub fn stale_committed(&self) -> &[LinkRef<S>] {
        &self.stale_committed
    }
}

/// Linear processor pipeline executor.
///
/// This is still a "low initiative" data structure, it must be driven by some
/// external sync engine that decides which links to process and which paths to
/// commit.  The executor only reports when a request doesn't make sense
/// against what it has.
pub struct LinearExecutor<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> {
    stages: StageRunner<S>,
    tracking: TrackingState<S>,

    /// Links whose stored artifacts have been loaded into the runner, and
    /// whether the store had any.
    loaded: HashMap<LinkRef<S>, bool>,

    provider: Arc<P>,
    store: Arc<X>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>
    LinearExecutor<S, P, X>
{
    /// Opens an executor over a store, reconciling what it holds with the
    /// pipeline.
    ///
    /// A store that has never been used starts every stage at `base_node`,
    /// which is then the oldest node the pipeline can roll back to until it's
    /// pruned past.  Otherwise the committed path's artifacts are loaded and
    /// stages that have no committed state or lag the committed node are
    /// brought level.  Uncommitted links are left where they are; one whose
    /// artifacts turn out to be missing or stale has those stages re-run when
    /// it's next processed.
    pub fn open(
        pipeline: StagePipeline<S>,
        provider: Arc<P>,
        store: Arc<X>,
        base_node: NodeRef<S>,
    ) -> Result<(Self, OpenReport<S>), GExecError> {
        let tracking = match store.load_tracking().map_err(GExecError::Storage)? {
            Some(tracking) => tracking,
            None => {
                let tracking = TrackingState::new_at(base_node);
                store
                    .store_tracking(&tracking)
                    .map_err(GExecError::Storage)?;
                tracking
            }
        };

        let mut exec = Self {
            stages: StageRunner::new(pipeline),
            tracking,
            loaded: HashMap::new(),
            provider,
            store,
        };

        let mut report = OpenReport::new();
        for lref in exec.committed_path().links().to_vec() {
            exec.hydrate_link(&lref)?;
            let missing = exec.stages.missing_stages(&lref);
            if !missing.is_empty() {
                exec.stages.mark_stale(lref.clone(), missing);
                report.stale_committed.push(lref);
            }
        }
        exec.reconcile_stages(&mut report)?;
        Ok((exec, report))
    }

    /// The node every stage has committed up to.
    pub fn committed_node(&self) -> &NodeRef<S> {
        self.tracking.committed_node()
    }

    /// The committed links that can still be rolled back, from the oldest.
    pub fn committed_path(&self) -> &LinkPath<S> {
        self.tracking.committed_path()
    }

    pub fn pipeline(&self) -> &StagePipeline<S> {
        self.stages.pipeline()
    }

    /// Whether every stage has accepted a link, committed or not.
    pub fn is_processed(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        // FIXME(trey): what's the purpose of this fn if we can eta-reduce it?
        self.is_usable(lref)
    }

    /// The artifact a stage produced for a processed link.
    pub fn get_artifact<A: ProcArtifact>(
        &mut self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Result<Option<Arc<A>>, GExecError> {
        self.hydrate_link(lref)?;
        Ok(self.stages.get_artifact(lref, proc_id))
    }

    /// Runs every stage on a link whose origin is reachable from the committed
    /// node, recording it if they all accept it.
    ///
    /// Processing a link every stage has already accepted is a no-op
    /// reporting acceptance; one some stages haven't (never run, or run by
    /// another version of the stage) has just those stages run.  A rejected
    /// link leaves nothing behind, so it can be asked about again.
    pub fn process_link(&mut self, lref: &LinkRef<S>) -> Result<LinkOutcome, GExecError> {
        self.hydrate_link(lref)?;
        let missing = self.stages.missing_stages(lref);
        if missing.is_empty() {
            return Ok(LinkOutcome::Accepted);
        }

        let endpoints = self.fetch_link_endpoints(lref)?;
        let path = self.path_to_origin(lref, endpoints.origin())?;
        let link = self.fetch_link(lref)?;

        let run = self.stages.run(lref, &link, &path, &missing)?;
        match run.outcome() {
            LinkOutcome::Accepted => {
                self.store_artifacts(run.produced())?;
                self.loaded.insert(lref.clone(), true);
            }
            LinkOutcome::Rejected { .. } => {
                // Whatever an earlier run had stored for it goes too, along
                // with anything that was built on it.
                self.forget_link(lref)?;
                self.sweep_from(endpoints.target())?;
            }
        }
        Ok(run.outcome())
    }

    /// Commits the path from the committed node through a processed link, in
    /// every stage.
    ///
    /// The path is the one with the fewest links to the link's origin.  The
    /// links stay recorded afterwards so the commit can be undone.
    pub fn commit_through(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        if self.tracking.is_committed(lref) {
            return Err(GExecError::LinkOnCommittedPath(format!("{lref:?}")));
        }
        if !self.is_usable(lref)? {
            return Err(GExecError::LinkNotProcessed(format!("{lref:?}")));
        }

        let endpoints = self.fetch_link_endpoints(lref)?;
        let mut path = self.path_to_origin(lref, endpoints.origin())?;
        let pushed = path.try_push_link(lref.clone(), &endpoints);
        debug_assert!(pushed, "gchain: path found to the link's own origin");

        for proc_id in self.get_proc_ids() {
            self.stages.commit_stage(proc_id, &path)?;
            self.set_stage_node(proc_id, path.terminal_node().clone())?;
        }

        self.tracking.extend_committed(&path);
        self.store_tracking()?;
        self.evict_to_committed();
        Ok(())
    }

    /// Rolls every stage back to a node on the committed path, undoing the
    /// links after it in reverse canonical order.
    ///
    /// The undone links stay recorded as uncommitted, so they can be committed
    /// again or built on.
    pub fn uncommit_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        let undone = self.tracking.committed_path_from(node)?;
        if undone.is_empty() {
            return Ok(());
        }
        self.stages.check_undoable(&undone)?;

        for proc_id in self.get_proc_ids().into_iter().rev() {
            self.stages.uncommit_stage(proc_id, &undone)?;
            self.set_stage_node(proc_id, node.clone())?;
        }

        self.tracking.truncate_committed_to(node)?;
        self.store_tracking()
    }

    /// Forgets an uncommitted link along with every link that was only
    /// reachable through it, returning everything forgotten.
    pub fn discard_link(&mut self, lref: &LinkRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        if self.tracking.is_committed(lref) {
            return Err(GExecError::LinkOnCommittedPath(format!("{lref:?}")));
        }
        if !self.is_present(lref)? {
            return Err(GExecError::LinkNotProcessed(format!("{lref:?}")));
        }

        let endpoints = self.fetch_link_endpoints(lref)?;
        self.forget_link(lref)?;
        let mut dropped = vec![lref.clone()];
        dropped.extend(self.sweep_from(endpoints.target())?);
        Ok(dropped)
    }

    /// Gives up the ability to roll back to before a node on the committed
    /// path, forgetting the links before it and everything hanging off them,
    /// and lets the stages discard what they kept for that.
    ///
    /// Returns every link forgotten.
    pub fn prune_upto(&mut self, node: &NodeRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        // The base moves first: if discarding is cut short, what's left
        // behind is unreachable from the new base and a later sweep finds it.
        let old_base = self.committed_path().base_node().clone();
        self.tracking.advance_base_to(node)?;
        self.store_tracking()?;

        let dropped = self.sweep_from(&old_base)?;
        self.stages.prune_upto(node)?;
        self.evict_to_committed();
        Ok(dropped)
    }

    fn get_proc_ids(&self) -> Vec<ProcId> {
        self.stages.pipeline().proc_ids().collect()
    }

    /// Fetches a link from the underlying provider and repackages the errors to
    /// gobble missing links.
    fn fetch_link(&self, lref: &LinkRef<S>) -> Result<Link<S>, GExecError> {
        self.provider
            .fetch_link(lref)?
            .ok_or_else(|| GExecError::MissingLink(format!("{lref:?}")))
    }

    /// Fetches the nodes a link connects, which is how the executor knows where
    /// the link sits relative to what it has processed.
    fn fetch_link_endpoints(&self, lref: &LinkRef<S>) -> Result<LinkEndpoints<S>, GExecError> {
        self.provider
            .fetch_link_endpoints(lref)?
            .ok_or_else(|| GExecError::MissingLinkEndpoints(format!("{lref:?}")))
    }

    /// Loads a link's stored artifacts into the runner if they aren't there
    /// already, reporting whether the store had any.
    fn hydrate_link(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        if let Some(present) = self.loaded.get(lref) {
            return Ok(*present);
        }

        let records = self
            .store
            .load_link_artifacts(lref)
            .map_err(GExecError::Storage)?;
        let present = !records.is_empty();
        for record in records {
            self.stages.insert_stored(record)?;
        }
        self.loaded.insert(lref.clone(), present);
        Ok(present)
    }

    /// Whether every stage has a current artifact for a link, which is what
    /// lets a path run through it.
    fn is_usable(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        // FIXME(trey): this fn seems cheap but is actually expensive since it might do IO
        self.hydrate_link(lref)?;
        Ok(self.stages.missing_stages(lref).is_empty())
    }

    /// Whether the store has anything at all for a link, without loading it.
    fn is_present(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        match self.loaded.get(lref) {
            Some(present) => Ok(*present),
            None => self
                .store
                .has_link_artifacts(lref)
                .map_err(GExecError::Storage),
        }
    }

    /// A path of usable links from the committed node to a link's origin,
    /// with the fewest links.
    fn path_to_origin(
        &mut self,
        lref: &LinkRef<S>,
        origin: &NodeRef<S>,
    ) -> Result<LinkPath<S>, GExecError> {
        let provider = Arc::clone(&self.provider);
        let committed = self.committed_node().clone();
        find_path(provider.as_ref(), &committed, origin, |l| self.is_usable(l))?
            .ok_or_else(|| GExecError::OriginUnreachable(format!("{lref:?}")))
    }

    /// Forgets every link that's reachable from a node but no longer from the
    /// committed path's base, returning them.
    fn sweep_from(&mut self, node: &NodeRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        let provider = Arc::clone(&self.provider);
        let base = self.committed_path().base_node().clone();
        let keep = find_reachable_links(provider.as_ref(), &base, |l| self.is_present(l))?;
        let candidates = find_reachable_links(provider.as_ref(), node, |l| self.is_present(l))?;

        let doomed: Vec<_> = candidates
            .into_iter()
            .filter(|l| !keep.contains(l))
            .collect();
        for lref in &doomed {
            self.forget_link(lref)?;
        }
        Ok(doomed)
    }

    /// Forgets a link everywhere it's tracked, letting each stage clean up
    /// after its artifact before the artifacts go.
    fn forget_link(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        self.hydrate_link(lref)?;
        self.stages.discard(lref)?;
        self.store
            .discard_link_artifacts(lref)
            .map_err(GExecError::Storage)?;
        self.loaded.remove(lref);
        Ok(())
    }

    /// Drops every loaded artifact except the committed path's.
    fn evict_to_committed(&mut self) {
        let keep: HashSet<_> = self.committed_path().links().iter().cloned().collect();
        self.stages.retain_links(&keep);
        self.loaded.retain(|lref, _| keep.contains(lref));
    }

    fn store_artifacts(&self, produced: &[ArtifactRecord<S>]) -> Result<(), GExecError> {
        for record in produced {
            self.store
                .store_artifact(record)
                .map_err(GExecError::Storage)?;
        }
        Ok(())
    }

    fn set_stage_node(&mut self, proc_id: ProcId, node: NodeRef<S>) -> Result<(), GExecError> {
        self.tracking.set_stage_node(proc_id, node);
        self.store_tracking()
    }

    fn store_tracking(&self) -> Result<(), GExecError> {
        self.store
            .store_tracking(&self.tracking)
            .map_err(GExecError::Storage)
    }

    /// Brings every stage's committed state level with the committed node.
    fn reconcile_stages(&mut self, report: &mut OpenReport<S>) -> Result<(), GExecError> {
        let committed = self.committed_node().clone();
        for proc_id in self.get_proc_ids() {
            match self.tracking.get_stage_node(proc_id).cloned() {
                None => {
                    self.stages.init_stage(proc_id, &committed)?;
                    report.initialized.push(proc_id);
                }
                Some(node) if node == committed => continue,
                Some(node) => {
                    if self.committed_path().get_node_index(&node).is_none() {
                        return Err(GExecError::StageDiverged {
                            proc_id,
                            node: format!("{node:?}"),
                        });
                    }
                    let behind = self.tracking.committed_path_from(&node)?;
                    self.stages.commit_stage(proc_id, &behind)?;
                    report.recommitted.push(proc_id);
                }
            }
            self.set_stage_node(proc_id, committed.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::mem_store::MemExecutorStore;
    use crate::test_support::*;

    type Exec = LinearExecutor<TestSpec, TestProvider, MemExecutorStore<TestSpec>>;

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

    /// Opens an executor at genesis node 1 with the given stages under the
    /// IDs "a", "b", ... in order.
    fn open(
        provider: &Arc<TestProvider>,
        store: &Arc<MemExecutorStore<TestSpec>>,
        procs: Vec<TestProc>,
    ) -> (Exec, OpenReport<TestSpec>) {
        LinearExecutor::open(
            pipeline_of(procs),
            Arc::clone(provider),
            Arc::clone(store),
            TestRef(1),
        )
        .expect("test: open executor")
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

    fn is_processed(exec: &mut Exec, lref: u8) -> bool {
        exec.is_processed(&TestRef(lref))
            .expect("test: check processed")
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
        assert!(is_processed(&mut exec, 11));
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
        assert!(
            exec.get_artifact::<FlagArtifact>(&TestRef(10), id("a"))
                .expect("test: fetch artifact")
                .is_some()
        );
    }

    #[test]
    fn test_processing_recorded_link_again_is_noop() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, _) = fresh(vec![proc]);
        take_events(&events);

        accept(&mut exec, 10);
        accept(&mut exec, 10);

        assert_eq!(take_events(&events), vec![ProcEvent::Process(TestRef(10))]);
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
        assert!(!is_processed(&mut exec, 10));
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
        assert!(is_processed(&mut exec, 11));
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
        assert!(is_processed(&mut exec, 20));
        assert!(!has_stored(&store, 11));
        assert!(has_stored(&store, 20));

        let err = exec.discard_link(&TestRef(10)).unwrap_err();
        assert!(matches!(err, GExecError::LinkNotProcessed(_)));
    }

    #[test]
    fn test_discard_refuses_committed_link() {
        let (mut exec, _, _) = fresh(vec![TestProc::new()]);
        accept(&mut exec, 10);
        exec.commit_through(&TestRef(10)).expect("test: commit");

        let err = exec.discard_link(&TestRef(10)).unwrap_err();
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
        assert!(is_processed(&mut exec, 12));
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
        assert!(report.stale_committed().is_empty());
        assert_eq!(exec.committed_node(), &TestRef(2));
        assert_eq!(exec.committed_path().links(), &refs(&[10]));
        assert!(is_processed(&mut exec, 11));
        assert!(is_processed(&mut exec, 30));
        assert!(
            exec.get_artifact::<FlagArtifact>(&TestRef(11), id("a"))
                .expect("test: fetch artifact")
                .is_some()
        );

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
        assert!(!is_processed(&mut exec, 20));
        accept(&mut exec, 20);
        assert!(is_processed(&mut exec, 20));
    }

    #[test]
    fn test_reopen_keeps_stale_committed_links_but_refuses_to_undo_them() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            accept(&mut exec, 10);
            accept(&mut exec, 11);
            exec.commit_through(&TestRef(10)).expect("test: commit");
        }

        let proc = TestProc::new().with_version(2);
        let events = proc.events();
        let (mut exec, report) = open(&provider, &store, vec![proc]);

        assert_eq!(report.stale_committed(), &refs(&[10]));
        assert!(take_events(&events).is_empty());
        assert_eq!(exec.committed_node(), &TestRef(2));
        accept(&mut exec, 11);
        assert_eq!(take_events(&events), vec![ProcEvent::Process(TestRef(11))]);

        let err = exec.uncommit_to(&TestRef(1)).unwrap_err();
        assert!(matches!(err, GExecError::StaleArtifact { proc_id, .. } if proc_id == id("a")));

        // Pruning past it clears the problem.
        exec.prune_upto(&TestRef(2)).expect("test: prune");
        accept(&mut exec, 30);
    }

    /// Only the committed path stays loaded across a commit; anything else
    /// comes back from the store when a path needs it.
    #[test]
    fn test_commit_evicts_uncommitted_artifacts_but_they_reload() {
        let (mut exec, _, _) = fresh(vec![TestProc::new()]);
        accept(&mut exec, 10);
        accept(&mut exec, 30);
        exec.commit_through(&TestRef(10)).expect("test: commit");

        assert_eq!(exec.loaded.keys().collect::<Vec<_>>(), vec![&TestRef(10)]);
        assert!(is_processed(&mut exec, 30));
        assert!(exec.loaded.contains_key(&TestRef(30)));
    }
}
