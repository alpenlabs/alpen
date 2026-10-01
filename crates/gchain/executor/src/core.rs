//! What an executor has on hand and the mechanics of keeping it in step with
//! the store.
//!
//! This is shared by an executor that's ready to be driven and one that still
//! has committed links to reprocess, so it promises neither: the stages may
//! sit at different nodes, and committed links may be missing artifacts.
//! Everything here is a step that leaves the store consistent with what's in
//! memory; which steps make sense when is up to whoever holds the core.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use strata_gchain_types::*;

use crate::config::StagePipeline;
use crate::errors::GExecError;
use crate::history::CommitWalker;
use crate::stage_runner::{LinkOutcome, StageRunner};
use crate::store::{ArtifactRecord, ExecutorStore};
use crate::tracking::{CommitIndex, TrackingState};
use crate::traverse::{find_path, find_reachable_links};

/// Where a node sits in the commit log.
struct LogCut<S: GChainSpec> {
    idx: CommitIndex,
    segment: LinkPath<S>,

    /// The node's position in the segment.
    pos: usize,
}

/// The stages that had to be brought level with the committed node.
pub(crate) struct LeveledStages {
    pub(crate) initialized: Vec<ProcId>,
    pub(crate) recommitted: Vec<ProcId>,
}

pub(crate) struct ExecutorCore<
    S: GChainSpec,
    P: ChainProvider<Spec = S>,
    X: ExecutorStore<Spec = S>,
> {
    stages: StageRunner<S>,

    /// Where the pipeline stands.
    ///
    /// Only ever mutated through [`Self::update_tracking`], which persists
    /// it, so it never drifts from what the store has.
    tracking: TrackingState<S>,

    /// Links whose stored artifacts have been loaded into the runner, and
    /// whether the store had any.
    loaded: HashMap<LinkRef<S>, bool>,

    provider: Arc<P>,
    store: Arc<X>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> ExecutorCore<S, P, X> {
    /// Opens a store, starting it off at `base_node` if it has never been
    /// used, and loads the committed path's artifacts from it.
    pub(crate) fn open(
        pipeline: StagePipeline<S>,
        provider: Arc<P>,
        store: Arc<X>,
        base_node: NodeRef<S>,
    ) -> Result<Self, GExecError> {
        let tracking = load_or_init_tracking(store.as_ref(), base_node)?;
        trim_commit_log(store.as_ref(), &tracking)?;

        let mut stages = StageRunner::new(pipeline);
        let loaded = load_path_artifacts(
            &mut stages,
            provider.as_ref(),
            store.as_ref(),
            tracking.committed_path(),
        )?;

        Ok(Self {
            stages,
            tracking,
            loaded,
            provider,
            store,
        })
    }

    pub(crate) fn committed_node(&self) -> &NodeRef<S> {
        self.tracking.committed_node()
    }

    pub(crate) fn committed_path(&self) -> &LinkPath<S> {
        self.tracking.committed_path()
    }

    pub(crate) fn tracking(&self) -> &TrackingState<S> {
        &self.tracking
    }

    pub(crate) fn stages(&self) -> &StageRunner<S> {
        &self.stages
    }

    pub(crate) fn store(&self) -> &X {
        self.store.as_ref()
    }

    pub(crate) fn get_proc_ids(&self) -> Vec<ProcId> {
        self.stages.pipeline().proc_ids().collect()
    }

    #[cfg(test)]
    pub(crate) fn check_loaded(&self, lref: &LinkRef<S>) -> bool {
        self.loaded.contains_key(lref)
    }

    /// The artifact a stage has for a link, loading the link's artifacts if
    /// they aren't loaded yet.
    pub(crate) fn get_artifact<A: ProcArtifact>(
        &mut self,
        lref: &LinkRef<S>,
        proc_id: ProcId,
    ) -> Result<Option<Arc<A>>, GExecError> {
        self.hydrate_link(lref)?;
        Ok(self.stages.get_artifact(lref, proc_id))
    }

    /// The stages with no current artifact for a link, loading the link's
    /// artifacts if they aren't loaded yet.
    pub(crate) fn get_missing_stages(
        &mut self,
        lref: &LinkRef<S>,
    ) -> Result<Vec<ProcId>, GExecError> {
        self.hydrate_link(lref)?;
        Ok(self.stages.missing_stages(lref))
    }

    /// Fetches the nodes a link connects, which is how the executor knows where
    /// the link sits relative to what it has processed.
    pub(crate) fn fetch_link_endpoints(
        &self,
        lref: &LinkRef<S>,
    ) -> Result<LinkEndpoints<S>, GExecError> {
        self.provider
            .fetch_link_endpoints(lref)?
            .ok_or_else(|| GExecError::missing_link_endpoints(lref))
    }

    /// Whether every stage has a current artifact for a link, which is what
    /// lets a path run through it.  Loads the link's artifacts on the way if
    /// they aren't loaded yet.
    pub(crate) fn check_usable(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        self.hydrate_link(lref)?;
        Ok(self.stages.missing_stages(lref).is_empty())
    }

    /// Whether the store has anything at all for a link, without loading it.
    pub(crate) fn check_present(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
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
    pub(crate) fn path_to_origin(
        &mut self,
        lref: &LinkRef<S>,
        origin: &NodeRef<S>,
    ) -> Result<LinkPath<S>, GExecError> {
        let provider = Arc::clone(&self.provider);
        let committed = self.committed_node().clone();
        find_path(provider.as_ref(), &committed, origin, |l| {
            self.check_usable(l)
        })?
        .ok_or_else(|| GExecError::origin_unreachable(lref))
    }

    /// Runs some stages on a link whose pre-state is reached by a path,
    /// storing what they produced if they all accepted it.
    ///
    /// A rejected link is left for the caller to clean up after.
    pub(crate) fn run_stages(
        &mut self,
        lref: &LinkRef<S>,
        path: &LinkPath<S>,
        stages: &[ProcId],
    ) -> Result<LinkOutcome, GExecError> {
        let link = self.fetch_link(lref)?;
        let run = self.stages.run(lref, &link, path, stages)?;
        if run.outcome() == LinkOutcome::Accepted {
            self.store_artifacts(run.produced())?;
            self.loaded.insert(lref.clone(), true);
        }
        Ok(run.outcome())
    }

    /// Forgets every link that's reachable from a node but no longer from the
    /// committed path's base, returning them.
    pub(crate) fn sweep_from(&mut self, node: &NodeRef<S>) -> Result<Vec<LinkRef<S>>, GExecError> {
        let provider = Arc::clone(&self.provider);
        let base = self.committed_path().base_node().clone();
        let keep = find_reachable_links(provider.as_ref(), &base, |l| self.check_present(l))?;
        let candidates = find_reachable_links(provider.as_ref(), node, |l| self.check_present(l))?;

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
    pub(crate) fn forget_link(&mut self, lref: &LinkRef<S>) -> Result<(), GExecError> {
        self.hydrate_link(lref)?;
        self.stages.discard(lref)?;
        self.store
            .discard_link_artifacts(lref)
            .map_err(GExecError::Storage)?;
        self.loaded.remove(lref);
        Ok(())
    }

    /// Drops every loaded artifact except the committed path's.
    pub(crate) fn evict_to_committed(&mut self) {
        let keep: HashSet<_> = self.committed_path().links().iter().cloned().collect();
        self.stages.retain_links(&keep);
        self.loaded.retain(|lref, _| keep.contains(lref));
    }

    /// Commits a path continuing from the committed node in every stage, as
    /// the next commit in the log.
    ///
    /// The segment goes in first: if the commit is cut short, it's beyond
    /// what the tracking state covers and is dropped on the next open.
    pub(crate) fn commit_path(&mut self, path: &LinkPath<S>) -> Result<(), GExecError> {
        self.store
            .store_commit_segment(self.tracking.next_commit(), path)
            .map_err(GExecError::Storage)?;

        for proc_id in self.get_proc_ids() {
            self.stages.commit_stage(proc_id, path)?;
            self.set_stage_node(proc_id, path.terminal_node().clone())?;
        }

        self.update_tracking(|tracking| {
            tracking.extend_committed(path);
            Ok(())
        })?;
        self.evict_to_committed();
        Ok(())
    }

    /// Checks that every stage could undo the committed links after a node,
    /// returning them.
    pub(crate) fn check_undoable_from(&self, node: &NodeRef<S>) -> Result<LinkPath<S>, GExecError> {
        let undone = self.tracking.committed_path_from(node)?;
        self.stages.check_undoable(&undone)?;
        Ok(undone)
    }

    /// Commits the committed links between two nodes in one stage, a commit
    /// at a time as they were first committed, leaving the stage at the
    /// later node.
    pub(crate) fn commit_stage_between(
        &mut self,
        proc_id: ProcId,
        from: &NodeRef<S>,
        to: &NodeRef<S>,
    ) -> Result<(), GExecError> {
        for segment in self.find_committed_segments_between(from, to)? {
            self.stages.commit_stage(proc_id, &segment)?;
            self.set_stage_node(proc_id, segment.terminal_node().clone())?;
        }
        Ok(())
    }

    /// Undoes the committed links between two nodes in one stage, a commit
    /// at a time from the newest, leaving the stage at the earlier node.
    pub(crate) fn uncommit_stage_between(
        &mut self,
        proc_id: ProcId,
        from: &NodeRef<S>,
        to: &NodeRef<S>,
    ) -> Result<(), GExecError> {
        let segments = self.find_committed_segments_between(from, to)?;
        for segment in segments.iter().rev() {
            self.stages.uncommit_stage(proc_id, segment)?;
            self.set_stage_node(proc_id, segment.base_node().clone())?;
        }
        Ok(())
    }

    /// Cuts the committed path and the commit log back to a node on the
    /// path.  The stages have to have been rolled back to it already.
    pub(crate) fn truncate_committed_to(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        self.check_on_committed_path(node)?;
        let cut = self.find_log_cut(node)?;
        let next_commit = match &cut {
            None => self.tracking.next_commit(),
            Some(cut) if cut.pos == 0 => cut.idx,
            Some(cut) => cut.idx.next(),
        };

        self.update_tracking(|tracking| tracking.truncate_committed_to(node, next_commit))?;

        if let Some(cut) = cut
            && cut.pos > 0
            && cut.pos < cut.segment.len()
        {
            self.store
                .store_commit_segment(cut.idx, &cut.segment.slice(0, cut.pos))
                .map_err(GExecError::Storage)?;
        }
        self.store
            .discard_commit_segments_from(next_commit)
            .map_err(GExecError::Storage)
    }

    /// Moves the base of the committed path and the start of the commit log
    /// forward to a node on the path, forgetting every link that can't be
    /// reached from it any more and returning them.
    pub(crate) fn advance_committed_base_to(
        &mut self,
        node: &NodeRef<S>,
    ) -> Result<Vec<LinkRef<S>>, GExecError> {
        self.check_on_committed_path(node)?;
        let cut = self.find_log_cut(node)?;
        let first_commit = match &cut {
            None => self.tracking.first_commit(),
            Some(cut) if cut.pos == cut.segment.len() => cut.idx.next(),
            Some(cut) => cut.idx,
        };

        // The base moves first: if discarding is cut short, what's left
        // behind is unreachable from the new base and a later sweep finds it.
        let old_base = self.committed_path().base_node().clone();
        self.update_tracking(|tracking| tracking.advance_base_to(node, first_commit))?;

        if let Some(cut) = cut
            && cut.pos > 0
            && cut.pos < cut.segment.len()
        {
            let kept = cut.segment.slice(cut.pos, cut.segment.len());
            self.store
                .store_commit_segment(cut.idx, &kept)
                .map_err(GExecError::Storage)?;
        }
        self.store
            .discard_commit_segments_before(first_commit)
            .map_err(GExecError::Storage)?;

        self.sweep_from(&old_base)
    }

    /// Lets every stage discard what it kept for rolling back to before a
    /// node.
    pub(crate) fn prune_stages_upto(&mut self, node: &NodeRef<S>) -> Result<(), GExecError> {
        self.stages.prune_upto(node)?;
        self.evict_to_committed();
        Ok(())
    }

    /// Brings every stage's committed state level with the committed node:
    /// a stage that has never been initialized is initialized there, and one
    /// that lags has the rest of the committed path committed.
    // TODO(trey): should this be moved to the ReprocExecutor?
    pub(crate) fn level_stages(&mut self) -> Result<LeveledStages, GExecError> {
        let mut leveled = LeveledStages {
            initialized: Vec::new(),
            recommitted: Vec::new(),
        };

        let committed = self.committed_node().clone();
        for proc_id in self.get_proc_ids() {
            match self.tracking.get_stage_node(proc_id).cloned() {
                None => {
                    self.stages.init_stage(proc_id, &committed)?;
                    self.set_stage_node(proc_id, committed.clone())?;
                    leveled.initialized.push(proc_id);
                }
                Some(node) if node == committed => {}
                Some(node) => {
                    if self.committed_path().get_node_index(&node).is_none() {
                        return Err(GExecError::stage_diverged(proc_id, &node));
                    }
                    self.commit_stage_between(proc_id, &node, &committed)?;
                    leveled.recommitted.push(proc_id);
                }
            }
        }

        Ok(leveled)
    }

    /// The commits covering the committed links between two nodes on the
    /// committed path, oldest first, with the ones at either end cut down to
    /// the part in between.
    // TODO(trey): should this be moved to the ReprocExecutor?
    fn find_committed_segments_between(
        &self,
        from: &NodeRef<S>,
        to: &NodeRef<S>,
    ) -> Result<Vec<LinkPath<S>>, GExecError> {
        if from == to {
            return Ok(Vec::new());
        }

        let mut segments = Vec::new();
        let mut reached_to = false;
        let mut walker = CommitWalker::from_tracking(self.store(), &self.tracking);
        while let Some(commit) = walker.next_segment()? {
            let mut segment = commit.into_path();
            if !reached_to {
                let Some(pos) = segment.get_node_index(to) else {
                    continue;
                };
                segment = segment.slice(0, pos);
                reached_to = true;
            }

            let from_pos = segment.get_node_index(from);
            if let Some(pos) = from_pos {
                segment = segment.slice(pos, segment.len());
            }
            if !segment.is_empty() {
                segments.push(segment);
            }
            if from_pos.is_some() {
                segments.reverse();
                return Ok(segments);
            }
        }

        Err(GExecError::corrupt_commit_log(from))
    }

    /// Finds the newest commit a node on the committed path is part of, if
    /// anything is committed.
    // TODO(trey): should this be moved to the ReprocExecutor?
    fn find_log_cut(&self, node: &NodeRef<S>) -> Result<Option<LogCut<S>>, GExecError> {
        let mut walker = CommitWalker::from_tracking(self.store(), &self.tracking);
        while let Some(commit) = walker.next_segment()? {
            if let Some(pos) = commit.path().get_node_index(node) {
                return Ok(Some(LogCut {
                    idx: commit.idx(),
                    segment: commit.into_path(),
                    pos,
                }));
            }
        }

        if self.committed_path().is_empty() {
            Ok(None)
        } else {
            Err(GExecError::corrupt_commit_log(node))
        }
    }

    fn check_on_committed_path(&self, node: &NodeRef<S>) -> Result<(), GExecError> {
        self.committed_path()
            .get_node_index(node)
            .is_some()
            .ok_or_else(|| GExecError::node_not_on_committed_path(node))
    }

    /// Fetches a link from the underlying provider and repackages the errors to
    /// gobble missing links.
    fn fetch_link(&self, lref: &LinkRef<S>) -> Result<Link<S>, GExecError> {
        self.provider
            .fetch_link(lref)?
            .ok_or_else(|| GExecError::missing_link(lref))
    }

    /// Loads a link's stored artifacts into the runner if they aren't there
    /// already, reporting whether the store had any.
    fn hydrate_link(&mut self, lref: &LinkRef<S>) -> Result<bool, GExecError> {
        if let Some(present) = self.loaded.get(lref) {
            return Ok(*present);
        }

        let present = load_link_artifacts(
            &mut self.stages,
            self.provider.as_ref(),
            self.store.as_ref(),
            lref,
        )?;
        self.loaded.insert(lref.clone(), present);
        Ok(present)
    }

    // FIXME(trey): this fn is dumb, make it take the record directly instead of
    // as a slice, do the loop on the outside
    fn store_artifacts(&self, produced: &[ArtifactRecord<S>]) -> Result<(), GExecError> {
        for record in produced {
            self.store
                .store_artifact(record)
                .map_err(GExecError::Storage)?;
        }
        Ok(())
    }

    fn set_stage_node(&mut self, proc_id: ProcId, node: NodeRef<S>) -> Result<(), GExecError> {
        self.update_tracking(|tracking| {
            tracking.set_stage_node(proc_id, node);
            Ok(())
        })
    }

    /// Applies a change to the tracking state and persists the result.
    ///
    /// This is the only way `self.tracking` gets mutated, so that no change
    /// can be made and then forgotten to be stored.  An update that fails
    /// must leave the state as it found it, since nothing is stored then.
    fn update_tracking(
        &mut self,
        update: impl FnOnce(&mut TrackingState<S>) -> Result<(), GExecError>,
    ) -> Result<(), GExecError> {
        // For safety, clone the tracking state and apply the changes to that,
        // only overwrite the field if the store is successful.
        //
        // TODO(trey): long term, improve the interface here so that this is
        // harder to fuck up
        let mut tracking = self.tracking.clone();
        update(&mut tracking)?;
        self.store
            .store_tracking(&tracking)
            .map_err(GExecError::Storage)?;
        self.tracking = tracking;
        Ok(())
    }

    /// Checks if the core's state is "clean", meaning that we've applied all
    /// the changes we needed to make to the database to open an executor over
    /// it.
    pub(crate) fn check_clean(&self) -> bool {
        self.get_proc_ids()
            .into_iter()
            .all(|id| self.tracking().get_stage_node(id) == Some(self.committed_node()))
    }
}

/// Loads where the pipeline stands, starting it off at `base_node` if the
/// store has never been used.
fn load_or_init_tracking<X: ExecutorStore>(
    store: &X,
    base_node: NodeRef<X::Spec>,
) -> Result<TrackingState<X::Spec>, GExecError> {
    if let Some(tracking) = store.load_tracking().map_err(GExecError::Storage)? {
        return Ok(tracking);
    }

    let tracking = TrackingState::new_at(base_node);
    store
        .store_tracking(&tracking)
        .map_err(GExecError::Storage)?;
    Ok(tracking)
}

/// Drops whatever the commit log has beyond what the tracking state covers,
/// which is what a commit, rollback, or prune that was cut short leaves
/// behind.
fn trim_commit_log<X: ExecutorStore>(
    store: &X,
    tracking: &TrackingState<X::Spec>,
) -> Result<(), GExecError> {
    store
        .discard_commit_segments_from(tracking.next_commit())
        .map_err(GExecError::Storage)?;
    store
        .discard_commit_segments_before(tracking.first_commit())
        .map_err(GExecError::Storage)?;

    let Some(last) = tracking.last_commit() else {
        return Ok(());
    };

    let committed = tracking.committed_node();
    let newest = load_commit_segment(store, last)?;
    if newest.terminal_node() != committed {
        let pos = newest
            .get_node_index(committed)
            .ok_or_else(|| GExecError::corrupt_commit_log(committed))?;
        store
            .store_commit_segment(last, &newest.slice(0, pos))
            .map_err(GExecError::Storage)?;
    }

    let first = tracking.first_commit();
    let base = tracking.committed_path().base_node();
    let oldest = load_commit_segment(store, first)?;
    if oldest.base_node() != base {
        let pos = oldest
            .get_node_index(base)
            .ok_or_else(|| GExecError::corrupt_commit_log(base))?;
        store
            .store_commit_segment(first, &oldest.slice(pos, oldest.len()))
            .map_err(GExecError::Storage)?;
    }
    Ok(())
}

fn load_commit_segment<X: ExecutorStore>(
    store: &X,
    idx: CommitIndex,
) -> Result<LinkPath<X::Spec>, GExecError> {
    store
        .load_commit_segment(idx)
        .map_err(GExecError::Storage)?
        .ok_or(GExecError::MissingCommitSegment(idx))
}

/// Loads the stored artifacts of every link on a path into a runner,
/// returning the links with whether the store had any for them.
fn load_path_artifacts<S: GChainSpec>(
    stages: &mut StageRunner<S>,
    provider: &impl ChainProvider<Spec = S>,
    store: &impl ExecutorStore<Spec = S>,
    path: &LinkPath<S>,
) -> Result<HashMap<LinkRef<S>, bool>, GExecError> {
    let mut loaded = HashMap::new();
    for lref in path.links() {
        let present = load_link_artifacts(stages, provider, store, lref)?;
        loaded.insert(lref.clone(), present);
    }
    Ok(loaded)
}

/// Loads a link's stored artifacts into a runner, reporting whether the store
/// had any.
///
/// The link's header is only fetched if some artifact came from an older
/// version of its stage, which the stage judges by it.
fn load_link_artifacts<S: GChainSpec>(
    stages: &mut StageRunner<S>,
    provider: &impl ChainProvider<Spec = S>,
    store: &impl ExecutorStore<Spec = S>,
    lref: &LinkRef<S>,
) -> Result<bool, GExecError> {
    let records = store
        .load_link_artifacts(lref)
        .map_err(GExecError::Storage)?;
    let present = !records.is_empty();

    let needs_header = records.iter().any(|r| stages.check_needs_header(r));
    let header = match needs_header {
        true => provider.fetch_link_header(lref)?,
        false => None,
    };
    for record in records {
        stages.insert_stored(record, header.as_ref())?;
    }
    Ok(present)
}
