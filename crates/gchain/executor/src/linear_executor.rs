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
//! Everything it tracks is mirrored in its [`ExecutorStore`]: artifacts for
//! every recorded link (committed ones too, so a commit can be undone), the
//! links themselves with their endpoints, the committed path, and how far each
//! stage has committed.  Reopening rebuilds the in-memory view from those.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::sync::Arc;

use strata_gchain_types::*;

use crate::artifact_cache::ArtifactCache;
use crate::config::StagePipeline;
use crate::errors::GExecError;
use crate::graph::LinkGraph;
use crate::schedule::LinkCoverage;
use crate::store::{ExecutorStore, LinkRecord};

/// The verdict on a link the executor was asked to process.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkOutcome {
    /// Every stage accepted the link; it's recorded and can be built on.
    Accepted,

    /// A stage rejected the link.  Nothing is recorded, and no later stage was
    /// run on it.
    Rejected { proc_id: ProcId },
}

/// What [`LinearExecutor::open`] did to bring the stored state in line with
/// the pipeline it was opened with.
pub struct OpenReport<S: GChainSpec> {
    initialized: Vec<ProcId>,
    recommitted: Vec<ProcId>,
    reprocessed: Vec<LinkRef<S>>,
    dropped: Vec<LinkRef<S>>,
    stale_committed: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> OpenReport<S> {
    fn new() -> Self {
        Self {
            initialized: Vec::new(),
            recommitted: Vec::new(),
            reprocessed: Vec::new(),
            dropped: Vec::new(),
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

    /// Uncommitted links that had stages re-run on them because their stored
    /// artifacts were missing or from another version of the stage.
    pub fn reprocessed(&self) -> &[LinkRef<S>] {
        &self.reprocessed
    }

    /// Uncommitted links that were forgotten, because a stage re-run on them
    /// rejected them, the provider no longer had them, or nothing could reach
    /// them any more.
    pub fn dropped(&self) -> &[LinkRef<S>] {
        &self.dropped
    }

    /// Committed links some stage has no usable artifact for, because it was
    /// stored by another version of the stage or the stage was added after
    /// the link was committed.  They stay committed, but the path can't be
    /// rolled back across them.
    pub fn stale_committed(&self) -> &[LinkRef<S>] {
        &self.stale_committed
    }
}

/// Where the pipeline's committed state sits.
struct CommittedState<S: GChainSpec> {
    /// The committed path, from the oldest node still rolled back to up to
    /// the committed node.
    path: LinkPath<S>,

    /// Where each stage has committed up to.  Level with the path's terminal
    /// once [`LinearExecutor::open`] returns.
    stages: BTreeMap<ProcId, NodeRef<S>>,

    /// Committed links with the stages whose artifact for them can't be used:
    /// either stored by another version of the stage, or never produced
    /// because the stage was added after the link was committed.
    stale_links: BTreeMap<LinkRef<S>, Vec<ProcId>>,
}

/// Linear processor pipeline executor.
///
/// This is still a "low initiative" data structure, it must be driven by some
/// external sync engine that decides which links to process and which paths to
/// commit.  The executor only reports when a request doesn't make sense
/// against what it has.
pub struct LinearExecutor<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> {
    pipeline: StagePipeline<S>,
    provider: Arc<P>,
    store: Arc<X>,
    cache: ArtifactCache<S>,
    graph: LinkGraph<S>,
    committed: CommittedState<S>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>
    LinearExecutor<S, P, X>
{
    /// Opens an executor over a store, reconciling what it holds with the
    /// pipeline.
    ///
    /// A store that has never been used starts every stage at `genesis`.
    /// Otherwise the committed path and processed links are restored, stages
    /// that have no committed state or lag the committed node are brought
    /// level, and uncommitted links whose artifacts are missing or stale have
    /// those stages re-run (which needs the provider to still have them).
    pub fn open(
        pipeline: StagePipeline<S>,
        provider: Arc<P>,
        store: Arc<X>,
        genesis: NodeRef<S>,
    ) -> Result<(Self, OpenReport<S>), GExecError> {
        let mut graph = LinkGraph::new();
        for record in store.load_links().map_err(GExecError::Storage)? {
            let (lref, endpoints) = record.into_parts();
            graph.insert(lref, endpoints);
        }

        let path = match store.load_committed_path().map_err(GExecError::Storage)? {
            Some(desc) => {
                let (base, links) = desc.into_parts();
                resolve_path(&graph, base, &links)?
            }
            None => {
                let path = LinkPath::new_at(genesis);
                store
                    .store_committed_path(&path.to_desc())
                    .map_err(GExecError::Storage)?;
                path
            }
        };

        let mut exec = Self {
            pipeline,
            provider,
            store,
            cache: ArtifactCache::new(),
            graph,
            committed: CommittedState {
                path,
                stages: BTreeMap::new(),
                stale_links: BTreeMap::new(),
            },
        };

        let mut report = OpenReport::new();
        let incomplete = exec.load_stored_artifacts()?;
        for lref in exec.committed.path.links() {
            if let Some(missing) = incomplete.get(lref) {
                exec.committed
                    .stale_links
                    .insert(lref.clone(), missing.clone());
                report.stale_committed.push(lref.clone());
            }
        }

        exec.reconcile_stages(&mut report)?;
        exec.restore_uncommitted_links(&incomplete, &mut report)?;
        Ok((exec, report))
    }

    /// The node every stage has committed up to.
    pub fn committed_node(&self) -> &NodeRef<S> {
        self.committed.path.terminal_node()
    }

    /// The committed links that can still be rolled back, from the oldest.
    pub fn committed_path(&self) -> &LinkPath<S> {
        &self.committed.path
    }

    pub fn pipeline(&self) -> &StagePipeline<S> {
        &self.pipeline
    }

    /// Whether a link has been processed and accepted, committed or not.
    pub fn is_processed(&self, lref: &LinkRef<S>) -> bool {
        self.graph.contains(lref)
    }

    /// The processed links departing from a node.
    pub fn links_from(&self, node: &NodeRef<S>) -> &[LinkRef<S>] {
        self.graph.links_from(node)
    }

    /// The artifact a stage produced for a processed link.
    pub fn get_artifact<A: ProcArtifact>(
        &self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Option<Arc<A>> {
        self.cache.get_artifact(lref, proc_id)
    }

    /// Runs every stage on a link whose origin is reachable from the committed
    /// node, recording it if they all accept it.
    ///
    /// Processing an already-recorded link is a no-op reporting acceptance.
    /// A rejected link leaves nothing behind, so it can be asked about again.
    pub fn process_link(&mut self, lref: &LinkRef<S>) -> Result<LinkOutcome, GExecError> {
        if self.graph.contains(lref) {
            return Ok(LinkOutcome::Accepted);
        }

        let endpoints = self.fetch_link_endpoints(lref)?;
        let path = self
            .graph
            .find_path(self.committed_node(), endpoints.origin())
            .ok_or_else(|| GExecError::OriginUnreachable(format!("{lref:?}")))?;
        let link = self.fetch_link(lref)?;

        let outcome = self.run_stages(lref, &link, &path, |_| true)?;
        match outcome {
            LinkOutcome::Accepted => {
                // Artifacts went in first, so the record is the last write
                // and a crash before it leaves only orphan artifacts.
                let record = LinkRecord::new(lref.clone(), endpoints);
                self.store
                    .store_link(&record)
                    .map_err(GExecError::Storage)?;
                let (lref, endpoints) = record.into_parts();
                self.graph.insert(lref, endpoints);
            }
            LinkOutcome::Rejected { .. } => {
                self.store
                    .discard_link_artifacts(lref)
                    .map_err(GExecError::Storage)?;
                self.cache.remove_link(lref);
            }
        }
        Ok(outcome)
    }

    /// Commits the path from the committed node through a processed link, in
    /// every stage.
    ///
    /// The path is the one with the fewest links to the link's origin.  The
    /// links stay recorded afterwards so the commit can be undone.
    pub fn commit_through(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        if self.committed.path.links().contains(lref) {
            return Err(GExecError::LinkOnCommittedPath(format!("{lref:?}")));
        }
        let endpoints = self
            .graph
            .endpoints(lref)
            .ok_or_else(|| GExecError::LinkNotProcessed(format!("{lref:?}")))?
            .clone();
        let mut path = self
            .graph
            .find_path(self.committed_node(), endpoints.origin())
            .ok_or_else(|| GExecError::OriginUnreachable(format!("{lref:?}")))?;
        let pushed = path.try_push_link(lref.clone(), &endpoints);
        debug_assert!(pushed, "gchain: path found to the link's own origin");
        let terminal = path.terminal_node().clone();

        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            let artifacts = cached_artifacts(&self.cache, proc_id, path.links())?;
            stage
                .chain_proc()
                .commit_outputs(&path, &artifacts)
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            self.store
                .store_committed_node(proc_id, &terminal)
                .map_err(GExecError::Storage)?;
            self.committed.stages.insert(proc_id, terminal.clone());
        }

        for lref in path.links() {
            let endpoints = self
                .graph
                .endpoints(lref)
                .expect("gchain: path links are recorded");
            let pushed = self.committed.path.try_push_link(lref.clone(), endpoints);
            debug_assert!(pushed, "gchain: committed path continues from its terminal");
        }
        self.store_committed_path()
    }

    /// Rolls every stage back to a node on the committed path, undoing the
    /// links after it in reverse canonical order.
    ///
    /// The undone links stay recorded as uncommitted, so they can be committed
    /// again or built on.
    pub fn uncommit_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        let idx = self.committed_path_index_of(node)?;
        let links = self.committed.path.links().to_vec();
        let (kept, undone) = links.split_at(idx);
        if undone.is_empty() {
            return Ok(());
        }

        for lref in undone {
            if let Some(missing) = self.committed.stale_links.get(lref) {
                return Err(GExecError::StaleArtifact {
                    link: format!("{lref:?}"),
                    proc_id: missing[0],
                });
            }
        }

        let suffix = resolve_path(&self.graph, node.clone(), undone)?;
        for stage in self.pipeline.iter_stages().rev() {
            let proc_id = stage.proc_id();
            let artifacts = cached_artifacts(&self.cache, proc_id, suffix.links())?;
            stage
                .chain_proc()
                .uncommit_outputs(&suffix, &artifacts)
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            self.store
                .store_committed_node(proc_id, node)
                .map_err(GExecError::Storage)?;
            self.committed.stages.insert(proc_id, node.clone());
        }

        let base = self.committed.path.base_node().clone();
        self.committed.path = resolve_path(&self.graph, base, kept)?;
        self.store_committed_path()
    }

    /// Forgets an uncommitted link along with every link that was only
    /// reachable through it, returning everything forgotten.
    pub fn discard_link(&mut self, lref: &LinkRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        if !self.graph.contains(lref) {
            return Err(GExecError::LinkNotProcessed(format!("{lref:?}")));
        }
        if self.committed.path.links().contains(lref) {
            return Err(GExecError::LinkOnCommittedPath(format!("{lref:?}")));
        }

        let mut dropped = vec![lref.clone()];
        self.discard_links(&dropped)?;
        dropped.extend(self.sweep_unreachable()?);
        Ok(dropped)
    }

    /// Gives up the ability to roll back to before a node on the committed
    /// path, forgetting the links before it and everything hanging off them,
    /// and lets the stages discard what they kept for that.
    ///
    /// Returns every link forgotten.
    pub fn prune_upto(&mut self, node: &NodeRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        let idx = self.committed_path_index_of(node)?;
        let links = self.committed.path.links().to_vec();
        let (pruned, kept) = links.split_at(idx);

        // Rebase the committed path first: if discarding is cut short, the
        // pruned links are unreachable from the new base and get swept on the
        // next open.
        self.committed.path = resolve_path(&self.graph, node.clone(), kept)?;
        self.store_committed_path()?;
        for lref in pruned {
            self.committed.stale_links.remove(lref);
        }

        self.discard_links(pruned)?;
        let mut dropped = pruned.to_vec();
        dropped.extend(self.sweep_unreachable()?);

        for stage in self.pipeline.iter_stages() {
            stage
                .chain_proc()
                .prune_state_upto(node)
                .map_err(|e| GExecError::Proc(stage.proc_id(), e))?;
        }
        Ok(dropped)
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

    /// Runs the selected stages on a link in canonical order, persisting and
    /// caching each artifact as it's produced, until one rejects the link.
    ///
    /// A rejection leaves the earlier stages' artifacts in place; the caller
    /// decides whether the link is being kept.
    fn run_stages(
        &mut self,
        lref: &LinkRef<S>,
        link: &Link<S>,
        path: &LinkPath<S>,
        should_run: impl Fn(ProcId) -> bool,
    ) -> Result<LinkOutcome, GExecError> {
        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            if !should_run(proc_id) {
                continue;
            }

            let coverage = link_coverage(&self.cache, &self.pipeline, lref, path);
            self.pipeline.schedule().check_ready(proc_id, &coverage)?;

            let artifact = stage
                .chain_proc()
                .process_link(lref, link, &self.cache, path)
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            if !artifact.is_link_valid() {
                return Ok(LinkOutcome::Rejected { proc_id });
            }

            let encoded = artifact
                .to_buf_dyn()
                .map_err(|e| GExecError::Proc(proc_id, e))?;
            let data = ProcessorArtifactData::new(stage.chain_proc().proc_version(), encoded);
            self.store
                .store_artifact(lref, proc_id, &data)
                .map_err(GExecError::Storage)?;
            self.cache.insert_artifact(lref.clone(), proc_id, artifact);
        }
        Ok(LinkOutcome::Accepted)
    }

    /// Loads every recorded link's artifacts into the cache, returning the
    /// stages each link is missing a current-version artifact for.
    fn load_stored_artifacts(&mut self) -> Result<BTreeMap<LinkRef<S>, Vec<ProcId>>, GExecError> {
        let lrefs: Vec<_> = self.graph.iter_links().map(|(l, _)| l.clone()).collect();
        let mut incomplete = BTreeMap::new();
        for lref in lrefs {
            let mut missing = Vec::new();
            for stage in self.pipeline.iter_stages() {
                let proc_id = stage.proc_id();
                let stored = self
                    .store
                    .load_artifact(&lref, proc_id)
                    .map_err(GExecError::Storage)?;
                match stored {
                    Some(data) if data.exec_version() == stage.chain_proc().proc_version() => {
                        let artifact = stage
                            .chain_proc()
                            .decode_artifact(&data)
                            .map_err(|e| GExecError::Proc(proc_id, e))?;
                        self.cache.insert_artifact(lref.clone(), proc_id, artifact);
                    }
                    _ => missing.push(proc_id),
                }
            }

            if !missing.is_empty() {
                incomplete.insert(lref, missing);
            }
        }
        Ok(incomplete)
    }

    /// Brings every stage's committed state level with the committed node.
    fn reconcile_stages(&mut self, report: &mut OpenReport<S>) -> Result<(), GExecError> {
        let committed = self.committed_node().clone();
        let nodes = path_nodes(&self.graph, &self.committed.path)?;

        for stage in self.pipeline.iter_stages() {
            let proc_id = stage.proc_id();
            let stored = self
                .store
                .load_committed_node(proc_id)
                .map_err(GExecError::Storage)?;
            match stored {
                None => {
                    stage
                        .chain_proc()
                        .on_init(&committed)
                        .map_err(|e| GExecError::Proc(proc_id, e))?;
                    report.initialized.push(proc_id);
                }
                Some(node) if node == committed => {}
                Some(node) => {
                    let idx = nodes.iter().position(|n| *n == node).ok_or_else(|| {
                        GExecError::StageDiverged {
                            proc_id,
                            node: format!("{node:?}"),
                        }
                    })?;
                    let behind = &self.committed.path.links()[idx..];
                    for lref in behind {
                        let stale_for_stage = self
                            .committed
                            .stale_links
                            .get(lref)
                            .is_some_and(|missing| missing.contains(&proc_id));
                        if stale_for_stage {
                            return Err(GExecError::StaleArtifact {
                                link: format!("{lref:?}"),
                                proc_id,
                            });
                        }
                    }

                    let path = resolve_path(&self.graph, node, behind)?;
                    let artifacts = cached_artifacts(&self.cache, proc_id, path.links())?;
                    stage
                        .chain_proc()
                        .commit_outputs(&path, &artifacts)
                        .map_err(|e| GExecError::Proc(proc_id, e))?;
                    report.recommitted.push(proc_id);
                }
            }

            self.store
                .store_committed_node(proc_id, &committed)
                .map_err(GExecError::Storage)?;
            self.committed.stages.insert(proc_id, committed.clone());
        }
        Ok(())
    }

    /// Re-runs missing stages on the uncommitted links reachable from the
    /// committed node, walking outwards so each link's path is complete
    /// before it's reached, and forgets whatever can't be restored.
    fn restore_uncommitted_links(
        &mut self,
        incomplete: &BTreeMap<LinkRef<S>, Vec<ProcId>>,
        report: &mut OpenReport<S>,
    ) -> Result<(), GExecError> {
        let committed = self.committed_node().clone();
        let mut dropped = Vec::new();
        let mut seen = HashSet::from([committed.clone()]);
        let mut queue = VecDeque::from([committed.clone()]);
        while let Some(node) = queue.pop_front() {
            for lref in self.graph.links_from(&node).to_vec() {
                let restored = match incomplete.get(&lref) {
                    Some(missing) => self.reprocess_link(&lref, missing, report)?,
                    None => true,
                };
                if !restored {
                    dropped.push(lref);
                    continue;
                }

                let target = self
                    .graph
                    .endpoints(&lref)
                    .expect("gchain: link is recorded")
                    .target();
                if seen.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }
        }
        self.discard_links(&dropped)?;

        // Links off the committed node's reach can't be reprocessed, since
        // there's no path to build their pre-state from.
        let reachable = self.graph.reachable_links_from(&committed);
        let stuck: Vec<_> = incomplete
            .keys()
            .filter(|l| self.graph.contains(l) && !reachable.contains(*l))
            .filter(|l| !self.committed.path.links().contains(l))
            .cloned()
            .collect();
        self.discard_links(&stuck)?;
        dropped.extend(stuck);

        dropped.extend(self.sweep_unreachable()?);
        report.dropped = dropped;
        Ok(())
    }

    /// Re-runs the stages a link is missing artifacts for, reporting whether
    /// the link is still good.
    fn reprocess_link(
        &mut self,
        lref: &LinkRef<S>,
        missing: &[ProcId],
        report: &mut OpenReport<S>,
    ) -> Result<bool, GExecError> {
        let Some(link) = self.provider.fetch_link(lref)? else {
            return Ok(false);
        };
        let origin = self
            .graph
            .endpoints(lref)
            .expect("gchain: link is recorded")
            .origin()
            .clone();
        let path = self
            .graph
            .find_path(self.committed_node(), &origin)
            .ok_or_else(|| GExecError::OriginUnreachable(format!("{lref:?}")))?;

        match self.run_stages(lref, &link, &path, |id| missing.contains(&id))? {
            LinkOutcome::Accepted => {
                report.reprocessed.push(lref.clone());
                Ok(true)
            }
            LinkOutcome::Rejected { .. } => Ok(false),
        }
    }

    /// Forgets links everywhere they're tracked, letting each stage clean up
    /// after its artifact first.
    fn discard_links(&mut self, lrefs: &[LinkRef<S>]) -> Result<(), GExecError> {
        for lref in lrefs {
            for stage in self.pipeline.iter_stages() {
                let proc_id = stage.proc_id();
                if let Some(artifact) = self.cache.get_artifact_dyn(lref, proc_id) {
                    stage
                        .chain_proc()
                        .preprune_artifact(lref, artifact.as_ref())
                        .map_err(|e| GExecError::Proc(proc_id, e))?;
                }
            }

            self.store
                .discard_link_artifacts(lref)
                .map_err(GExecError::Storage)?;
            self.store.discard_link(lref).map_err(GExecError::Storage)?;
            self.cache.remove_link(lref);
            self.graph.remove(lref);
        }
        Ok(())
    }

    /// Forgets every link that nothing reaches from the committed path's base
    /// any more, returning them.
    fn sweep_unreachable(&mut self) -> Result<Vec<LinkRef<S>>, GExecError> {
        let keep = self
            .graph
            .reachable_links_from(self.committed.path.base_node());
        let doomed: Vec<_> = self
            .graph
            .iter_links()
            .map(|(l, _)| l.clone())
            .filter(|l| !keep.contains(l))
            .collect();
        self.discard_links(&doomed)?;
        Ok(doomed)
    }

    /// The position of a node along the committed path, counting the base as
    /// zero.
    fn committed_path_index_of(&self, node: &NodeRef<S>) -> Result<usize, GExecError> {
        path_nodes(&self.graph, &self.committed.path)?
            .iter()
            .position(|n| n == node)
            .ok_or_else(|| GExecError::NodeNotOnCommittedPath(format!("{node:?}")))
    }

    fn store_committed_path(&self) -> Result<(), GExecError> {
        self.store
            .store_committed_path(&self.committed.path.to_desc())
            .map_err(GExecError::Storage)
    }
}

/// Resolves a stored path description against the recorded links, checking
/// that they connect.
fn resolve_path<S: GChainSpec>(
    graph: &LinkGraph<S>,
    base: NodeRef<S>,
    links: &[LinkRef<S>],
) -> Result<LinkPath<S>, GExecError> {
    let mut path = LinkPath::new_at(base);
    for lref in links {
        let connected = graph
            .endpoints(lref)
            .is_some_and(|e| path.try_push_link(lref.clone(), e));
        if !connected {
            return Err(GExecError::CorruptCommittedPath(format!("{lref:?}")));
        }
    }
    Ok(path)
}

/// The nodes along a path, from its base to its terminal.
fn path_nodes<S: GChainSpec>(
    graph: &LinkGraph<S>,
    path: &LinkPath<S>,
) -> Result<Vec<NodeRef<S>>, GExecError> {
    let mut nodes = vec![path.base_node().clone()];
    for lref in path.links() {
        let endpoints = graph
            .endpoints(lref)
            .ok_or_else(|| GExecError::CorruptCommittedPath(format!("{lref:?}")))?;
        nodes.push(endpoints.target().clone());
    }
    Ok(nodes)
}

/// Collects a stage's cached artifacts for a run of links, in order.
fn cached_artifacts<S: GChainSpec>(
    cache: &ArtifactCache<S>,
    proc_id: ProcId,
    links: &[LinkRef<S>],
) -> Result<Vec<Arc<dyn DynProcArtifact>>, GExecError> {
    links
        .iter()
        .map(|lref| {
            cache
                .get_artifact_dyn(lref, proc_id)
                .cloned()
                .ok_or_else(|| GExecError::MissingArtifact {
                    link: format!("{lref:?}"),
                    proc_id,
                })
        })
        .collect()
}

/// Works out which stages have covered a link and the path to its origin from
/// what's in the cache.
fn link_coverage<S: GChainSpec>(
    cache: &ArtifactCache<S>,
    pipeline: &StagePipeline<S>,
    lref: &LinkRef<S>,
    path: &LinkPath<S>,
) -> LinkCoverage {
    let mut coverage = LinkCoverage::new();
    for stage in pipeline.iter_stages() {
        let proc_id = stage.proc_id();
        let accepted_cur = cache
            .get_artifact_dyn(lref, proc_id)
            .is_some_and(|a| a.is_link_valid());
        if accepted_cur {
            coverage.mark_cur(proc_id);
        }

        let covers_path = path
            .links()
            .iter()
            .all(|l| cache.get_artifact_dyn(l, proc_id).is_some());
        if covers_path {
            coverage.mark_path(proc_id);
        }
    }
    coverage
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::config::PipelineBuilder;
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

    fn no_deps() -> ProcDeps {
        ProcDeps::new(Vec::new(), Vec::new())
    }

    /// Opens an executor at genesis node 1 with the given stages under the
    /// IDs "a", "b", ... in order.
    fn open(
        provider: &Arc<TestProvider>,
        store: &Arc<MemExecutorStore<TestSpec>>,
        procs: Vec<TestProc>,
    ) -> (Exec, OpenReport<TestSpec>) {
        let mut builder = PipelineBuilder::new();
        for (idx, proc) in procs.into_iter().enumerate() {
            let name = char::from(b'a' + idx as u8).to_string();
            builder = builder
                .add_stage(id(&name), proc, no_deps())
                .expect("test: add stage");
        }
        LinearExecutor::open(
            builder.build(),
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

    #[test]
    fn test_fresh_open_inits_every_stage_at_genesis() {
        let proc = TestProc::new();
        let events = proc.events();
        let (exec, _, store) = fresh(vec![proc]);

        assert_eq!(take_events(&events), vec![ProcEvent::Init(TestRef(1))]);
        assert_eq!(exec.committed_node(), &TestRef(1));
        assert!(exec.committed_path().is_empty());
        assert_eq!(
            store.load_committed_path().expect("test: load path"),
            Some(PathDesc::new(TestRef(1), Vec::new()))
        );
    }

    #[test]
    fn test_process_and_commit_round_trip() {
        let proc = TestProc::new();
        let events = proc.events();
        let (mut exec, _, store) = fresh(vec![proc]);
        take_events(&events);

        accept(&mut exec, 10);
        accept(&mut exec, 11);
        assert!(exec.is_processed(&TestRef(11)));
        assert_eq!(exec.links_from(&TestRef(2)), &[TestRef(11)]);
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
        assert_eq!(
            store.load_committed_node(id("a")).expect("test: load node"),
            Some(TestRef(3))
        );
        assert_eq!(
            store.load_committed_path().expect("test: load path"),
            Some(PathDesc::new(TestRef(1), refs(&[10, 11])))
        );
        // Committed links keep their artifacts so the commit can be undone.
        assert!(
            exec.get_artifact::<FlagArtifact>(&TestRef(10), id("a"))
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
        assert!(!exec.is_processed(&TestRef(10)));
        assert!(store.load_links().expect("test: load links").is_empty());
        assert!(
            store
                .load_artifact(&TestRef(10), id("a"))
                .expect("test: load artifact")
                .is_none()
        );
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
        assert!(exec.is_processed(&TestRef(11)));
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
        assert_eq!(
            store.load_committed_node(id("b")).expect("test: load node"),
            Some(TestRef(2))
        );

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
        assert!(exec.is_processed(&TestRef(20)));
        assert_eq!(store.load_links().expect("test: load links").len(), 1);

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
        assert!(exec.is_processed(&TestRef(12)));
        assert_eq!(
            store.load_committed_path().expect("test: load path"),
            Some(PathDesc::new(TestRef(3), Vec::new()))
        );

        // Nothing before the new base can be rolled back to any more.
        let err = exec.uncommit_to(&TestRef(1)).unwrap_err();
        assert!(matches!(err, GExecError::NodeNotOnCommittedPath(_)));
    }

    #[test]
    fn test_reopen_restores_links_without_reprocessing() {
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
        assert!(report.reprocessed().is_empty());
        assert!(report.dropped().is_empty());
        assert_eq!(exec.committed_node(), &TestRef(2));
        assert_eq!(exec.committed_path().links(), &refs(&[10]));
        assert!(exec.is_processed(&TestRef(11)));
        assert!(exec.is_processed(&TestRef(30)));
        assert!(
            exec.get_artifact::<FlagArtifact>(&TestRef(11), id("a"))
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
        store
            .store_committed_node(id("a"), &TestRef(2))
            .expect("test: store node");

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
        assert_eq!(
            store.load_committed_node(id("a")).expect("test: load node"),
            Some(TestRef(3))
        );
        assert_eq!(
            store.load_committed_node(id("b")).expect("test: load node"),
            Some(TestRef(3))
        );
    }

    #[test]
    fn test_reopen_reruns_stale_and_missing_stages_on_uncommitted_links() {
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
        let (exec, report) = open(&provider, &store, vec![first, second]);

        assert_eq!(report.reprocessed(), &refs(&[10, 11]));
        assert!(report.dropped().is_empty());
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
                ProcEvent::Init(TestRef(1)),
                ProcEvent::Process(TestRef(10)),
                ProcEvent::Process(TestRef(11)),
            ]
        );
        assert!(exec.is_processed(&TestRef(11)));
        assert_eq!(
            store
                .load_artifact(&TestRef(10), id("a"))
                .expect("test: load artifact")
                .map(|d| d.exec_version()),
            Some(ProcVersion::from(2))
        );
    }

    /// A link the new version of a stage rejects is dropped, and so is
    /// everything that was built on it.
    #[test]
    fn test_reopen_drops_links_rejected_on_rerun() {
        let provider = provider();
        let store = Arc::new(MemExecutorStore::new());
        {
            let (mut exec, _) = open(&provider, &store, vec![TestProc::new()]);
            for lref in [10, 11, 30, 20] {
                accept(&mut exec, lref);
            }
        }

        let proc = TestProc::new().with_version(2).rejecting([10]);
        let (exec, report) = open(&provider, &store, vec![proc]);

        let mut dropped = report.dropped().to_vec();
        dropped.sort();
        assert_eq!(dropped, refs(&[10, 11, 30]));
        assert_eq!(report.reprocessed(), &refs(&[20]));
        assert!(exec.is_processed(&TestRef(20)));
        assert!(!exec.is_processed(&TestRef(11)));
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
        assert_eq!(report.reprocessed(), &refs(&[11]));
        assert_eq!(take_events(&events), vec![ProcEvent::Process(TestRef(11))]);
        assert_eq!(exec.committed_node(), &TestRef(2));

        let err = exec.uncommit_to(&TestRef(1)).unwrap_err();
        assert!(matches!(err, GExecError::StaleArtifact { proc_id, .. } if proc_id == id("a")));

        // Pruning past it clears the problem.
        exec.prune_upto(&TestRef(2)).expect("test: prune");
        accept(&mut exec, 30);
    }
}
