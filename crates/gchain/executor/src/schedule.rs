//! Reasoning about when a stage may run.
//!
//! A stage's [`ProcDeps`] say which other stages' artifacts it reads: some for
//! the link being processed, some along the path to the link's origin.  This
//! module turns those into two checks that any executor can apply: at setup,
//! that the deps can be satisfied at all by the pipeline's canonical order, and
//! per link, that they're satisfied right now.  The checks are on IDs and deps
//! alone, so they don't care how an executor stores artifacts or schedules
//! work.

use std::collections::{BTreeMap, BTreeSet};

use strata_gchain_types::*;

use crate::errors::GExecError;

/// The stages of a pipeline in canonical order with their validated deps.
///
/// Canonical order is the order stages run for a single link when run
/// serially.  Every current-link dep points at an earlier stage, so running in
/// this order always finds them in place.  Built with
/// [`StageScheduleBuilder`].
pub(crate) struct StageSchedule {
    stages: Vec<(ProcId, ProcDeps)>,
    by_id: BTreeMap<ProcId, usize>,
}

impl StageSchedule {
    /// A stage's position in canonical order.
    pub(crate) fn index_of(&self, proc_id: ProcId) -> Option<usize> {
        self.by_id.get(&proc_id).copied()
    }

    fn deps(&self, proc_id: ProcId) -> Option<&ProcDeps> {
        self.index_of(proc_id).map(|idx| &self.stages[idx].1)
    }

    /// Checks that every dep of a stage is covered for a link.
    ///
    /// Reports the first unmet dep, which is what an executor needs to explain
    /// why it can't run the stage yet.
    ///
    /// # Panics
    ///
    /// If the stage isn't in the schedule, since an executor only asks about
    /// stages it registered.
    pub(crate) fn check_ready(
        &self,
        proc_id: ProcId,
        coverage: &LinkCoverage,
    ) -> Result<(), GExecError> {
        let deps = self
            .deps(proc_id)
            .expect("gchain: readiness check for unregistered stage");

        let unmet_cur = deps.cur_node().iter().find(|dep| !coverage.has_cur(**dep));
        let unmet_path = deps
            .prev_node()
            .iter()
            .find(|dep| !coverage.has_path(**dep));

        match unmet_cur.or(unmet_path) {
            Some(dep) => Err(GExecError::UnmetDep {
                stage: proc_id,
                dep: *dep,
            }),
            None => Ok(()),
        }
    }
}

/// Assembles a [`StageSchedule`] one stage at a time, in canonical order.
///
/// Each stage is validated as it's added, so the builder only ever holds a
/// consistent prefix of the schedule.
pub(crate) struct StageScheduleBuilder {
    stages: Vec<(ProcId, ProcDeps)>,
    by_id: BTreeMap<ProcId, usize>,
}

impl StageScheduleBuilder {
    pub(crate) fn new() -> Self {
        Self {
            stages: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    /// Appends a stage after every stage added so far.
    ///
    /// Rejects a duplicate ID (a dep naming one wouldn't say which stage it
    /// meant) and any dep on a stage that hasn't been added yet, since a
    /// current-link dep on a later stage could never be satisfied when running
    /// in canonical order.  A path dep may also name the stage itself, which
    /// is the normal case for a validating stage building on its own earlier
    /// output.
    pub(crate) fn add_stage(
        &mut self,
        proc_id: ProcId,
        deps: ProcDeps,
    ) -> Result<&mut Self, GExecError> {
        if self.by_id.contains_key(&proc_id) {
            return Err(GExecError::DuplicateProc(proc_id));
        }

        let unregistered_cur = deps
            .cur_node()
            .iter()
            .find(|dep| !self.by_id.contains_key(dep));
        let unregistered_path = deps
            .prev_node()
            .iter()
            .find(|dep| **dep != proc_id && !self.by_id.contains_key(dep));
        if let Some(dep) = unregistered_cur.or(unregistered_path) {
            return Err(GExecError::DepNotRegistered {
                stage: proc_id,
                dep: *dep,
            });
        }

        self.by_id.insert(proc_id, self.stages.len());
        self.stages.push((proc_id, deps));
        Ok(self)
    }

    pub(crate) fn build(self) -> StageSchedule {
        StageSchedule {
            stages: self.stages,
            by_id: self.by_id,
        }
    }
}

impl Default for StageScheduleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Which stages have covered a link and the path to its origin.
///
/// An executor fills this in from wherever it keeps artifacts before asking
/// the schedule whether a stage may run.  The link and path are fixed by the
/// executor, so the coverage only says which stages have handled them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LinkCoverage {
    cur: BTreeSet<ProcId>,
    path: BTreeSet<ProcId>,
}

impl LinkCoverage {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records that a stage has accepted the link.
    pub(crate) fn mark_cur(&mut self, proc_id: ProcId) {
        self.cur.insert(proc_id);
    }

    /// Records that a stage has artifacts for every link on the path from its
    /// committed node to the link's origin.
    pub(crate) fn mark_path(&mut self, proc_id: ProcId) {
        self.path.insert(proc_id);
    }

    pub(crate) fn has_cur(&self, proc_id: ProcId) -> bool {
        self.cur.contains(&proc_id)
    }

    pub(crate) fn has_path(&self, proc_id: ProcId) -> bool {
        self.path.contains(&proc_id)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::test_support::expect_err;

    fn id(s: &str) -> ProcId {
        ProcId::from_str(s).expect("test: parse ProcId")
    }

    fn deps(cur: &[&str], prev: &[&str]) -> ProcDeps {
        ProcDeps::new(
            cur.iter().map(|s| id(s)).collect(),
            prev.iter().map(|s| id(s)).collect(),
        )
    }

    fn schedule(stages: &[(&str, ProcDeps)]) -> Result<StageSchedule, GExecError> {
        let mut builder = StageScheduleBuilder::new();
        for (name, deps) in stages {
            builder.add_stage(id(name), deps.clone())?;
        }
        Ok(builder.build())
    }

    fn coverage(cur: &[&str], path: &[&str]) -> LinkCoverage {
        let mut cov = LinkCoverage::new();
        for s in cur {
            cov.mark_cur(id(s));
        }
        for s in path {
            cov.mark_path(id(s));
        }
        cov
    }

    #[test]
    fn test_schedule_keeps_canonical_order() {
        let sched = schedule(&[
            ("exec", deps(&[], &["exec"])),
            ("index", deps(&["exec"], &["exec"])),
        ])
        .expect("test: build schedule");

        assert_eq!(sched.index_of(id("exec")), Some(0));
        assert_eq!(sched.index_of(id("index")), Some(1));
        assert_eq!(sched.index_of(id("absent")), None);
    }

    #[test]
    fn test_duplicate_proc_ids_are_rejected() {
        let err = expect_err(
            schedule(&[
                ("dup", deps(&[], &[])),
                ("other", deps(&[], &[])),
                ("dup", deps(&[], &[])),
            ]),
            "duplicate to be rejected",
        );
        assert!(matches!(err, GExecError::DuplicateProc(p) if p == id("dup")));
    }

    /// A dep has to name a stage added before it: an unknown one can never be
    /// satisfied, and a current-link dep on a later stage would wait forever
    /// when stages run in canonical order.  Adding stages one at a time is
    /// what catches both.
    #[test]
    fn test_deps_must_name_earlier_stages() {
        let cases = [
            (deps(&["ghost"], &[]), "ghost"),
            (deps(&[], &["ghost"]), "ghost"),
            (deps(&["b"], &[]), "b"),
            (deps(&[], &["b"]), "b"),
        ];
        for (a_deps, dep) in cases {
            let err = expect_err(
                schedule(&[("a", a_deps), ("b", deps(&[], &[]))]),
                "dep on a stage not yet added to be rejected",
            );
            assert!(
                matches!(err, GExecError::DepNotRegistered { stage, dep: d } if stage == id("a") && d == id(dep)),
                "test: got {err:?}"
            );
        }
    }

    /// A stage that builds on its own earlier output is the normal case for a
    /// validating stage, so a self dep along the path is fine.
    #[test]
    fn test_path_dep_on_self_is_allowed() {
        schedule(&[("a", deps(&[], &["a"])), ("b", deps(&["a"], &["a", "b"]))])
            .expect("test: build schedule");
    }

    #[test]
    fn test_check_ready_reports_first_unmet_dep() {
        let sched = schedule(&[
            ("exec", deps(&[], &[])),
            ("index", deps(&["exec"], &["exec"])),
        ])
        .expect("test: build schedule");

        sched
            .check_ready(id("exec"), &LinkCoverage::new())
            .expect("test: no deps is ready");
        sched
            .check_ready(id("index"), &coverage(&["exec"], &["exec"]))
            .expect("test: covered deps are ready");

        for cov in [coverage(&[], &["exec"]), coverage(&["exec"], &[])] {
            let err = sched.check_ready(id("index"), &cov).unwrap_err();
            assert!(
                matches!(err, GExecError::UnmetDep { stage, dep } if stage == id("index") && dep == id("exec"))
            );
        }
    }
}
