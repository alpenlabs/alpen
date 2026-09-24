//! Describing a pipeline of processor stages to an executor.

use std::sync::Arc;

use strata_gchain_types::*;

use crate::errors::GExecError;
use crate::process::{GChainProcDyn, ProcShim};
use crate::schedule::{StageSchedule, StageScheduleBuilder};

/// A processor stage as registered in a pipeline.
pub(crate) struct Stage<S: GChainSpec> {
    chain_proc: Arc<dyn GChainProcDyn<S>>,
}

impl<S: GChainSpec> Stage<S> {
    pub(crate) fn proc_id(&self) -> ProcId {
        self.chain_proc.proc_id()
    }

    pub(crate) fn chain_proc(&self) -> &dyn GChainProcDyn<S> {
        self.chain_proc.as_ref()
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
    pub(crate) fn iter_stages(&self) -> impl DoubleEndedIterator<Item = &Stage<S>> {
        self.stages.iter()
    }

    /// The stages' IDs in canonical order.
    pub fn proc_ids(&self) -> impl DoubleEndedIterator<Item = ProcId> {
        self.stages.iter().map(Stage::proc_id)
    }

    /// Looks up a stage by the ID it's registered under.
    pub(crate) fn get_stage(&self, proc_id: ProcId) -> Option<&Stage<S>> {
        self.schedule.index_of(proc_id).map(|idx| &self.stages[idx])
    }

    pub(crate) fn schedule(&self) -> &StageSchedule {
        &self.schedule
    }
}

/// Assembles a [`StagePipeline`] one stage at a time, in canonical order.
///
/// This is what pairs a stage with the ID it's registered under, so the two
/// can't drift apart.  Deps are validated as each stage is added: IDs must be
/// unique, and a dep must name a stage already added (or, for a path dep, the
/// stage itself).
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
        self.schedule.add_stage(proc_id, deps)?;
        self.stages.push(Stage {
            chain_proc: Arc::new(ProcShim::new(proc_id, proc)),
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

    /// The builder is what pairs each stage with its ID and position, and it
    /// surfaces the schedule's dep checks (which have their own tests).
    #[test]
    fn test_builder_registers_stages_in_order_and_checks_deps() {
        let pipeline = PipelineBuilder::<TestSpec>::new()
            .add_stage(id("third"), TestProc::new(), no_deps())
            .expect("test: add stage")
            .add_stage(id("first"), TestProc::new(), no_deps())
            .expect("test: add stage")
            .build();

        let ids: Vec<_> = pipeline.proc_ids().collect();
        assert_eq!(ids, vec![id("third"), id("first")]);
        let found = pipeline.get_stage(id("first")).expect("test: find stage");
        assert_eq!(found.proc_id(), id("first"));
        assert!(pipeline.get_stage(id("absent")).is_none());

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
