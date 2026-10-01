//! Reprocessing committed links a stage no longer stands behind.
//!
//! Opening a store with newer stages than it was written by can find
//! committed links whose artifacts the stages don't accept any more.  Each
//! such stage gets a plan: the committed links it has to process again, from
//! the node it can resume at up to the committed node.  Applying a plan rolls
//! just that stage back with the artifacts it committed with, runs it over
//! the links again, and commits what that produced.
//!
//! Until every plan is applied there's no executor to drive, only a
//! [`PendingExecutor`] that knows what's left to do.

use std::collections::{HashSet, VecDeque};
use std::mem;

use strata_gchain_types::*;

use crate::core::ExecutorCore;
use crate::errors::GExecError;
use crate::history::{CommitWalker, CommittedStep, ResumeSearch, StepJudge, find_resume_path};
use crate::open::*;
use crate::stage_runner::{LinkOutcome, StageRunner};
use crate::store::ExecutorStore;

/// The committed links a stage has to process again.
pub struct StagePlan<S: GChainSpec> {
    proc_id: ProcId,
    path: LinkPath<S>,
}

impl<S: GChainSpec> StagePlan<S> {
    pub fn proc_id(&self) -> ProcId {
        self.proc_id
    }

    /// The links to process again, as the path from the node the stage
    /// resumes at.
    pub fn path(&self) -> &LinkPath<S> {
        &self.path
    }
}

/// A committed link that a stage rejected when processing it again.
///
/// The pipeline was rolled back to the link's origin, and the link forgotten
/// along with everything only reachable through it.
pub struct RejectedCommit<S: GChainSpec> {
    lref: LinkRef<S>,
    proc_id: ProcId,
    dropped: Vec<LinkRef<S>>,
}

impl<S: GChainSpec> RejectedCommit<S> {
    pub fn lref(&self) -> &LinkRef<S> {
        &self.lref
    }

    /// The stage that rejected the link.
    pub fn proc_id(&self) -> ProcId {
        self.proc_id
    }

    /// Every link forgotten, the rejected one included.
    pub fn dropped(&self) -> &[LinkRef<S>] {
        &self.dropped
    }
}

/// Executor over a store with committed links that have to be reprocessed
/// before it can be driven.
///
/// The plans are applied a stage at a time in canonical order, since a stage
/// builds on what the ones it depends on produce.  When to do that is up to
/// the caller; [`Self::finish`] applies whatever is left and hands over the
/// executor.
pub struct ReprocExecutor<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>> {
    core: ExecutorCore<S, P, X>,
    plans: VecDeque<StagePlan<S>>,

    // TODO(trey): move these to a reproc log structure, maybe also plans
    reprocessed: Vec<StagePlan<S>>,
    rejected: Vec<RejectedCommit<S>>,
}

impl<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>
    ReprocExecutor<S, P, X>
{
    pub(crate) fn new(core: ExecutorCore<S, P, X>, plans: Vec<StagePlan<S>>) -> Self {
        Self {
            core,
            plans: plans.into(),
            reprocessed: Vec::new(),
            rejected: Vec::new(),
        }
    }

    /// The node the pipeline is committed up to.
    pub fn committed_node(&self) -> &NodeRef<S> {
        self.core.committed_node()
    }

    /// The committed links that can still be rolled back, from the oldest.
    pub fn committed_path(&self) -> &LinkPath<S> {
        self.core.committed_path()
    }

    /// The plans still to be applied, in the order they will be.
    pub fn plans(&self) -> impl Iterator<Item = &StagePlan<S>> {
        self.plans.iter()
    }

    /// Committed links that were rejected so far.
    pub fn rejected(&self) -> &[RejectedCommit<S>] {
        &self.rejected
    }

    /// Applies the next stage's plan, returning the stage if there was one
    /// left.
    ///
    /// If the stage rejects a link, the whole pipeline is rolled back to the
    /// link's origin and the plans left are cut short to match.
    pub fn apply_next_stage(&mut self) -> Result<Option<ProcId>, GExecError> {
        let Some(plan) = self.plans.pop_front() else {
            return Ok(None);
        };
        let proc_id = plan.proc_id;

        match reprocess_stage(&mut self.core, &plan)? {
            None => self.reprocessed.push(plan),
            Some(rejection) => {
                let dropped = roll_back_rejected(&mut self.core, proc_id, &rejection)?;
                self.reprocessed.push(StagePlan {
                    proc_id,
                    path: plan.path.slice(0, rejection.plan_pos),
                });
                self.rejected.push(RejectedCommit {
                    lref: rejection.lref,
                    proc_id,
                    dropped,
                });
                self.cut_plans_to_committed();
            }
        }

        Ok(Some(proc_id))
    }

    /// Applies the plans that are left and hands over the executor.
    pub fn finish(mut self) -> Result<LinearExecutorReadyOutput<S, P, X>, GExecError> {
        while self.apply_next_stage()?.is_some() {
            // TODO(trey): add logging about what we're doing here
        }
        finish_open(self.core, self.reprocessed, self.rejected)
    }

    /// Cuts the plans that are left down to the links still committed after
    /// a rollback.
    fn cut_plans_to_committed(&mut self) {
        let committed = self.core.committed_path();
        let plans = mem::take(&mut self.plans);
        self.plans = plans
            .into_iter()
            .filter_map(|plan| cut_plan_to(plan, committed))
            .collect();
    }
}

/// Cuts a plan down to the part of it that's on the committed path, if any
/// of it is.
fn cut_plan_to<S: GChainSpec>(plan: StagePlan<S>, committed: &LinkPath<S>) -> Option<StagePlan<S>> {
    let resume_pos = committed.get_node_index(plan.path.base_node())?;
    let path = committed.slice(resume_pos, committed.len());
    (!path.is_empty()).then_some(StagePlan {
        proc_id: plan.proc_id,
        path,
    })
}

/// Judges committed links for one stage by the artifacts loaded for them.
// FIXME(trey): this type feels convoluted and context-specific, maybe we should
// a type that represents the precomputed dep graph
struct StageJudge<'c, S: GChainSpec> {
    stages: &'c StageRunner<S>,
    proc_id: ProcId,

    /// Links the stages this one depends on are reprocessing, which makes
    /// what it built on their artifacts out of date.
    forced: HashSet<LinkRef<S>>,
}

impl<S: GChainSpec> StepJudge<S> for StageJudge<'_, S> {
    fn check_step_acceptable(&mut self, step: &CommittedStep<S>) -> Result<bool, GExecError> {
        let lref = step.lref();
        Ok(!self.forced.contains(lref) && self.stages.has_current(lref, self.proc_id))
    }

    fn check_can_resume_at(&mut self, node: &NodeRef<S>) -> Result<bool, GExecError> {
        self.stages.check_can_resume_at(self.proc_id, node)
    }
}

/// Works out which committed links each stage has to process again, in
/// canonical order.
///
/// A stage that has never been initialized has no history to redo, and no
/// stage redoes anything from before the node it was initialized at.
pub(crate) fn plan_reprocessing<
    S: GChainSpec,
    P: ChainProvider<Spec = S>,
    X: ExecutorStore<Spec = S>,
>(
    core: &ExecutorCore<S, P, X>,
) -> Result<Vec<StagePlan<S>>, GExecError> {
    let tracking = core.tracking();
    let mut plans: Vec<StagePlan<S>> = Vec::new();

    for proc_id in core.get_proc_ids() {
        let Some(floor) = tracking.get_stage_floor(proc_id) else {
            continue;
        };

        let mut judge = StageJudge {
            stages: core.stages(),
            proc_id,
            forced: collect_dep_links(core.stages(), proc_id, &plans),
        };

        let walker = CommitWalker::from_tracking(core.store(), tracking);
        match find_resume_path(walker, tracking.committed_node(), floor, &mut judge)? {
            ResumeSearch::UpToDate => {}
            ResumeSearch::ResumeAt(path) => plans.push(StagePlan { proc_id, path }),
            ResumeSearch::NoResumePoint => return Err(GExecError::NoResumePoint(proc_id)),
        }
    }

    Ok(plans)
}

/// The links being reprocessed by the stages a stage depends on.
fn collect_dep_links<S: GChainSpec>(
    stages: &StageRunner<S>,
    proc_id: ProcId,
    plans: &[StagePlan<S>],
) -> HashSet<LinkRef<S>> {
    let deps = stages
        .pipeline()
        .schedule()
        .get_deps(proc_id)
        .expect("gchain: stage is in the schedule");

    plans
        .iter()
        .filter(|plan| plan.proc_id != proc_id)
        .filter(|plan| {
            deps.cur_node().contains(&plan.proc_id) || deps.prev_node().contains(&plan.proc_id)
        })
        .flat_map(|plan| plan.path.links().iter().cloned())
        .collect()
}

/// A committed link a stage rejected while reprocessing.
struct Rejection<S: GChainSpec> {
    lref: LinkRef<S>,
    origin: NodeRef<S>,
    target: NodeRef<S>,

    /// The link's position in the stage's plan.
    plan_pos: usize,
}

/// Rolls a stage back to where its plan resumes, runs it over the plan's
/// links, and commits it back up to the committed node.
///
/// If the stage rejects a link, it's left committed up to the link's origin
/// and the rejection is returned.
fn reprocess_stage<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>(
    core: &mut ExecutorCore<S, P, X>,
    plan: &StagePlan<S>,
) -> Result<Option<Rejection<S>>, GExecError> {
    let proc_id = plan.proc_id;
    let committed = core.committed_path().clone();

    let resume_node = plan.path.base_node();
    let resume_pos = committed
        .get_node_index(resume_node)
        .ok_or_else(|| GExecError::node_not_on_committed_path(resume_node))?;
    let stage_node = core
        .tracking()
        .get_stage_node(proc_id)
        .cloned()
        .expect("gchain: planned stage not initialized");
    let stage_pos = committed
        .get_node_index(&stage_node)
        .ok_or_else(|| GExecError::stage_diverged(proc_id, &stage_node))?;

    // A stage that was already behind where it resumes stays where it is,
    // and has what's in between committed along with the rest.
    let rewind_pos = resume_pos.min(stage_pos);
    let rewind_node = &committed.nodes()[rewind_pos];
    core.uncommit_stage_between(proc_id, rewind_node, &stage_node)?;

    for (plan_pos, lref) in plan.path.links().iter().enumerate() {
        let origin_pos = resume_pos + plan_pos;
        let pre_path = committed.slice(rewind_pos, origin_pos);
        let outcome = core.run_stages(lref, &pre_path, &[proc_id])?;

        if matches!(outcome, LinkOutcome::Rejected { .. }) {
            let origin = &committed.nodes()[origin_pos];
            core.commit_stage_between(proc_id, rewind_node, origin)?;
            return Ok(Some(Rejection {
                lref: lref.clone(),
                origin: origin.clone(),
                target: committed.nodes()[origin_pos + 1].clone(),
                plan_pos,
            }));
        }
    }

    core.commit_stage_between(proc_id, rewind_node, committed.terminal_node())?;
    Ok(None)
}

/// Rolls the pipeline back to the origin of a committed link a stage
/// rejected, then forgets the link and everything only reachable through
/// it, returning what was forgotten.
///
/// The rejecting stage is at the origin already.  Stages that were never
/// initialized have nothing to roll back.
fn roll_back_rejected<S: GChainSpec, P: ChainProvider<Spec = S>, X: ExecutorStore<Spec = S>>(
    core: &mut ExecutorCore<S, P, X>,
    rejecting: ProcId,
    rejection: &Rejection<S>,
) -> Result<Vec<LinkRef<S>>, GExecError> {
    let committed = core.committed_path().clone();
    let origin = &rejection.origin;
    let origin_pos = committed
        .get_node_index(origin)
        .ok_or_else(|| GExecError::node_not_on_committed_path(origin))?;

    for proc_id in core.get_proc_ids().into_iter().rev() {
        if proc_id == rejecting {
            continue;
        }
        let Some(node) = core.tracking().get_stage_node(proc_id).cloned() else {
            continue;
        };

        let pos = committed
            .get_node_index(&node)
            .ok_or_else(|| GExecError::stage_diverged(proc_id, &node))?;
        if pos > origin_pos {
            core.uncommit_stage_between(proc_id, origin, &node)?;
        }
    }

    core.truncate_committed_to(origin)?;
    core.forget_link(&rejection.lref)?;
    let mut dropped = vec![rejection.lref.clone()];
    dropped.extend(core.sweep_from(&rejection.target)?);
    core.evict_to_committed();

    Ok(dropped)
}
