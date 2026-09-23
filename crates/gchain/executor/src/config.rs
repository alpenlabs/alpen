//! Describing a pipeline of processor stages to an executor.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::errors::GExecError;
use crate::process::{GChainProcDyn, ProcShim};
use crate::schedule::{StageSchedule, StageScheduleBuilder};

/// A processor stage with the exec control data it was registered with.
pub struct Stage<S: GChainSpec> {
    chain_proc: Arc<dyn GChainProcDyn<S>>,
    deps: ProcDeps,
}

impl<S: GChainSpec> Stage<S> {
    pub fn proc_id(&self) -> ProcId {
        self.chain_proc.proc_id()
    }

    pub fn chain_proc(&self) -> &dyn GChainProcDyn<S> {
        self.chain_proc.as_ref()
    }

    pub fn deps(&self) -> &ProcDeps {
        &self.deps
    }
}

/// Pipeline of processor stages in canonical order with their schedule.
///
/// The stages are held in canonical order and the schedule only indexes into
/// them, so there's no second copy of the ordering that could disagree with
/// the first.  Built with [`PipelineBuilder`].
pub struct StagePipeline<S: GChainSpec> {
    stages: Vec<Stage<S>>,
    schedule: StageSchedule,
}

impl<S: GChainSpec> StagePipeline<S> {
    /// Returns an iterator over the stages in canonical order.
    pub fn iter_stages(&self) -> impl DoubleEndedIterator<Item = &Stage<S>> {
        self.stages.iter()
    }

    /// Looks up a stage by the ID it's registered under.
    pub fn get_stage(&self, proc_id: ProcId) -> Option<&Stage<S>> {
        self.schedule.index_of(proc_id).map(|idx| &self.stages[idx])
    }

    pub fn schedule(&self) -> &StageSchedule {
        &self.schedule
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

/// Assembles a [`StagePipeline`] one stage at a time, in canonical order.
///
/// This is what pairs a stage with the ID it's registered under, so the two
/// can't drift apart.  Deps are validated as each stage is added (see
/// [`StageScheduleBuilder::add_stage`]).
pub struct PipelineBuilder<S: GChainSpec> {
    stages: Vec<Stage<S>>,
    schedule: StageScheduleBuilder,
}

impl<S: GChainSpec> PipelineBuilder<S> {
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
            schedule: StageScheduleBuilder::new(),
        }
    }

    /// Appends a stage under an ID, after every stage added so far.
    pub fn add_stage(
        mut self,
        proc_id: ProcId,
        proc: impl GChainProc<Spec = S>,
        deps: ProcDeps,
    ) -> Result<Self, GExecError> {
        self.schedule.add_stage(proc_id, deps.clone())?;
        self.stages.push(Stage {
            chain_proc: Arc::new(ProcShim::new(proc_id, proc)),
            deps,
        });
        Ok(self)
    }

    pub fn build(self) -> StagePipeline<S> {
        StagePipeline {
            stages: self.stages,
            schedule: self.schedule.build(),
        }
    }
}

impl<S: GChainSpec> Default for PipelineBuilder<S> {
    fn default() -> Self {
        Self::new()
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

    fn no_deps() -> ProcDeps {
        ProcDeps::new(Vec::new(), Vec::new())
    }

    fn proc_ids(pipeline: &StagePipeline<TestSpec>) -> Vec<ProcId> {
        pipeline.iter_stages().map(Stage::proc_id).collect()
    }

    fn pipeline(names: &[&str]) -> StagePipeline<TestSpec> {
        let mut builder = PipelineBuilder::new();
        for name in names {
            builder = builder
                .add_stage(id(name), TestProc::new(), no_deps())
                .expect("test: add stage");
        }
        builder.build()
    }

    #[test]
    fn test_iter_stages_follows_insertion_order() {
        let pipeline = pipeline(&["third", "first", "second"]);
        assert_eq!(
            proc_ids(&pipeline),
            vec![id("third"), id("first"), id("second")]
        );
    }

    #[test]
    fn test_get_stage_finds_stage_by_id() {
        let pipeline = pipeline(&["first", "second"]);

        let found = pipeline.get_stage(id("second")).expect("test: find stage");
        assert_eq!(found.proc_id(), id("second"));
        assert!(pipeline.get_stage(id("absent")).is_none());
    }

    /// The builder validates each stage against the ones added before it.
    #[test]
    fn test_add_stage_rejects_bad_deps() {
        let err = expect_err(
            PipelineBuilder::<TestSpec>::new()
                .add_stage(id("dup"), TestProc::new(), no_deps())
                .expect("test: add stage")
                .add_stage(id("dup"), TestProc::new(), no_deps()),
            "duplicate to be rejected",
        );
        assert!(matches!(err, GExecError::DuplicateProc(p) if p == id("dup")));

        let err = expect_err(
            PipelineBuilder::<TestSpec>::new().add_stage(
                id("a"),
                TestProc::new(),
                ProcDeps::new(vec![id("b")], Vec::new()),
            ),
            "forward dep to be rejected",
        );
        assert!(matches!(err, GExecError::DepNotRegistered { .. }));
    }
}
