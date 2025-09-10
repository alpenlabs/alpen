use std::collections::{hash_map::Entry, HashMap, HashSet};

use strata_db::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use tracing::{info, warn};

use crate::{errors::ProvingTaskError, status::ProvingTaskStatus};

/// Manages tasks and their states for proving operations.
#[derive(Debug, Clone)]
pub(crate) struct TaskTracker {
    /// A map of task IDs to their statuses.
    tasks: HashMap<ProofKey, ProvingTaskStatus>,
    /// A map of task IDs that have failed (transiently) to their retry counter.
    /// Such a task will be retried for up to the configured max retry counter times.
    transient_failed_tasks: HashMap<ProofKey, u64>,
    /// A map of task IDs to their dependencies that have not yet been proven.
    pending_dependencies: HashMap<ProofKey, HashSet<ProofKey>>,
    /// Count of the tasks that are in progress
    in_progress_tasks: HashMap<ProofZkVm, usize>,
    /// List of ZkVm for which the task is created
    vms: Vec<ProofZkVm>,
}

impl TaskTracker {
    /// Creates a new `TaskTracker` instance.
    pub(crate) fn new() -> Self {
        let mut vms = vec![];

        #[cfg(feature = "sp1")]
        {
            vms.push(ProofZkVm::SP1);
        }

        #[cfg(feature = "risc0")]
        {
            vms.push(ProofZkVm::Risc0);
        }

        #[cfg(all(not(feature = "risc0"), not(feature = "sp1")))]
        {
            vms.push(ProofZkVm::Native);
        }

        TaskTracker {
            tasks: HashMap::new(),
            transient_failed_tasks: HashMap::new(),
            pending_dependencies: HashMap::new(),
            in_progress_tasks: HashMap::new(),
            vms,
        }
    }

    pub(crate) fn get_in_progress_tasks(&self) -> &HashMap<ProofZkVm, usize> {
        &self.in_progress_tasks
    }

    pub(crate) fn get_retriable_tasks(&self) -> HashMap<ProofKey, u64> {
        let transient_failures = self
            .get_tasks_by_status(|status| matches!(status, ProvingTaskStatus::TransientFailure));

        let mut retriable_tasks = HashMap::new();
        for task in transient_failures {
            retriable_tasks.insert(task, *self.transient_failed_tasks.get(&task).unwrap_or(&0));
        }

        retriable_tasks
    }

    pub(crate) fn get_waiting_for_dependencies_tasks(&self) -> Vec<ProofKey> {
        self.get_tasks_by_status(|status| {
            matches!(status, ProvingTaskStatus::WaitingForDependencies)
        })
    }

    pub(crate) fn create_tasks(
        &mut self,
        proof_id: ProofContext,
        deps: Vec<ProofContext>,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        info!(?proof_id, "Creating task for");
        let mut tasks = Vec::with_capacity(self.vms.len());
        // Insert tasks for each configured host
        let vms = &self.vms.clone();
        for host in vms {
            let task = ProofKey::new(proof_id, *host);
            tasks.push(task);
            let dep_tasks: Vec<_> = deps.iter().map(|&dep| ProofKey::new(dep, *host)).collect();
            self.insert_task(task, &dep_tasks, db)?;
        }

        Ok(tasks)
    }

    /// Inserts a new task with the given dependencies.
    ///
    /// - If no dependencies are provided, the task is marked as `Pending`.
    /// - If dependencies are provided, the task is marked as `WaitingForDependencies`.
    ///
    /// Returns an error if the task already exists.
    pub(crate) fn insert_task(
        &mut self,
        id: ProofKey,
        deps: &[ProofKey],
        db: &ProofDBSled,
    ) -> Result<(), ProvingTaskError> {
        if self.tasks.contains_key(&id) {
            return Err(ProvingTaskError::TaskAlreadyFound(id));
        }

        // Gather dependencies that are not completed
        let mut pending_deps = Vec::with_capacity(deps.len());
        for &dep in deps {
            let proof = db
                .get_proof(&dep)
                .map_err(ProvingTaskError::DatabaseError)?;
            match proof {
                Some(_) => {}
                None => {
                    pending_deps.push(dep);
                }
            }
        }

        if pending_deps.is_empty() {
            self.tasks.insert(id, ProvingTaskStatus::Pending);
        } else {
            self.pending_dependencies
                .insert(id, HashSet::from_iter(pending_deps));
            self.tasks
                .insert(id, ProvingTaskStatus::WaitingForDependencies);
        };

        Ok(())
    }

    /// Retrieves the status of a task by its ID.
    ///
    /// Returns an error if the task does not exist.
    pub(crate) fn get_task(&self, id: ProofKey) -> Result<&ProvingTaskStatus, ProvingTaskError> {
        self.tasks
            .get(&id)
            .ok_or(ProvingTaskError::TaskNotFound(id))
    }

    /// Updates the status of a task.
    ///
    /// - Allows valid transitions as per the state machine.
    /// - Automatically resolves dependencies if a task is completed.
    /// - Handles transient failures with the configurable limit.
    ///
    /// Returns an error for invalid transitions or if the task does not exist.
    pub(crate) fn update_status(
        &mut self,
        id: ProofKey,
        new_status: ProvingTaskStatus,
        max_retry_counter: u64,
    ) -> Result<(), ProvingTaskError> {
        if let Some(status) = self.tasks.get_mut(&id) {
            // Check for valid status transitions
            status.transition(new_status.clone())?;

            // Handle the new status.
            match new_status {
                ProvingTaskStatus::ProvingInProgress => {
                    // Just mark the task as in progress.
                    *self.in_progress_tasks.entry(*id.host()).or_insert(0) += 1;
                }
                ProvingTaskStatus::Completed => {
                    // Decrement value if key exists, or insert with a default value of 1
                    *self.in_progress_tasks.entry(*id.host()).or_insert(0) -= 1;

                    // Resolve dependencies for other tasks
                    let mut tasks_to_update = vec![];
                    for (dependent_task, deps) in self.pending_dependencies.iter_mut() {
                        if deps.remove(&id) && deps.is_empty() {
                            tasks_to_update.push(*dependent_task);
                        }
                    }

                    for task in tasks_to_update {
                        self.pending_dependencies.remove(&task);
                        if let Some(task_status) = self.tasks.get_mut(&task) {
                            task_status.transition(ProvingTaskStatus::Pending)?;
                        } else {
                            warn!(%task, "failed to find dependent task")
                        }
                    }

                    // Clean up state to not bloat the memory as the task has been completed.
                    self.tasks.remove(&id);
                    self.transient_failed_tasks.remove(&id);
                }
                ProvingTaskStatus::TransientFailure => {
                    // Decrement value if key exists, or insert with a default value of 1
                    *self.in_progress_tasks.entry(*id.host()).or_insert(0) -= 1;

                    // Check the retry counter against the max retry limit or just increment.
                    let retry_counter = self.transient_failed_tasks.entry(id);
                    match retry_counter {
                        Entry::Occupied(mut entry) => {
                            if *entry.get() >= max_retry_counter {
                                // If the task has reached the retry limit, transition to Failed.
                                status.transition(ProvingTaskStatus::Failed)?;
                                self.transient_failed_tasks.remove(&id);
                            } else {
                                *entry.get_mut() += 1;
                            }
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(1);
                        }
                    };
                }
                ProvingTaskStatus::Failed => {
                    // The task has failed permanently and is not retriable, so we clean up failed
                    // task counter entry.
                    self.transient_failed_tasks.remove(&id);

                    // If the dependency has failed, the task that depends on the dependency should
                    // also be marked as Failed
                    // Otherwise it will stuck on WaitingForDependencies
                    for (dependent_task, deps) in self.pending_dependencies.iter_mut() {
                        if deps.remove(&id) {
                            if let Some(task_status) = self.tasks.get_mut(dependent_task) {
                                task_status.transition(ProvingTaskStatus::Failed)?;
                            } else {
                                warn!(%dependent_task, "failed to find dependent task")
                            }
                        }
                    }
                }
                _ => {}
            };
            Ok(())
        } else {
            Err(ProvingTaskError::TaskNotFound(id))
        }
    }

    /// Filters and retrieves a list of `ProofKey` references for tasks whose status
    /// matches the given filter function.
    ///
    /// # Example
    ///
    /// ```rust
    /// let task_tracker = TaskTracker::new();
    /// let pending_tasks =
    ///     task_tracker.get_tasks_by_status(|status| matches!(status, ProvingTaskStatus::Pending));
    /// ```
    pub(crate) fn get_tasks_by_status<F>(&self, filter_fn: F) -> Vec<ProofKey>
    where
        F: Fn(&ProvingTaskStatus) -> bool,
    {
        self.tasks
            .iter()
            .filter_map(|(proof_key, task)| {
                if filter_fn(task) {
                    Some(*proof_key) // Only return the `proof_key` if the task matches the filter
                } else {
                    None
                }
            })
            .collect()
    }

    /// Generates a report of task statuses and their counts across all tasks.
    pub(crate) fn generate_report(&self) -> HashMap<String, usize> {
        let mut report: HashMap<String, usize> = HashMap::new();

        for status in self.tasks.values() {
            *report.entry(format!("{status:?}")).or_insert(0) += 1;
        }

        report
    }

    /// Clears the internal state of the [`TaskTracker`], should be used only in testing.
    #[cfg(test)]
    pub(crate) fn clear_state(&mut self) {
        self.tasks.clear();
        self.in_progress_tasks.clear();
        self.pending_dependencies.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::SledDbConfig;
    use strata_primitives::{
        l2::L2BlockCommitment,
        proof::{ProofContext, ProofZkVm},
    };
    use strata_test_utils::ArbitraryGenerator;
    use typed_sled::SledDb;

    use super::*;

    // Helper function to generate test L1 block IDs
    fn gen_task_with_deps(n: u64) -> (ProofKey, Vec<ProofKey>) {
        let mut deps = Vec::with_capacity(n as usize);
        let host = ProofZkVm::Native;
        let mut gen = ArbitraryGenerator::new();

        let start: L2BlockCommitment = gen.generate();
        let end: L2BlockCommitment = gen.generate();
        for _ in 0..n {
            let start = gen.generate();
            let end = gen.generate();
            let id = ProofContext::EvmEeStf(start, end);
            let key = ProofKey::new(id, host);
            deps.push(key);
        }

        let id = ProofContext::ClStf(start, end);
        let key = ProofKey::new(id, host);

        (key, deps)
    }

    fn setup_db() -> ProofDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = Arc::new(SledDb::new(db).unwrap());
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        ProofDBSled::new(sled_db, config).unwrap()
    }

    #[test]
    fn test_insert_task_no_dependencies() {
        let mut tracker = TaskTracker::new();
        let (id, _) = gen_task_with_deps(0);
        let db = setup_db();

        tracker.insert_task(id, &[], &db).unwrap();
        assert!(
            matches!(tracker.get_task(id), Ok(&ProvingTaskStatus::Pending)),
            "Task with no dependencies should be Pending"
        );
    }

    #[test]
    fn test_insert_task_with_dependencies() {
        let mut tracker = TaskTracker::new();
        let (id, deps) = gen_task_with_deps(2);
        let db = setup_db();

        for dep in &deps {
            tracker.insert_task(*dep, &[], &db).unwrap();
        }
        tracker.insert_task(id, &deps.clone(), &db).unwrap();
        assert!(
            matches!(
                tracker.get_task(id),
                Ok(&ProvingTaskStatus::WaitingForDependencies)
            ),
            "Task with dependencies should be WaitingForDependencies"
        );
    }

    #[test]
    fn test_task_not_found_error() {
        let mut tracker = TaskTracker::new();
        let (id, _) = gen_task_with_deps(0);
        let max_retry_counter = 15u64;

        let result = tracker.update_status(id, ProvingTaskStatus::Pending, max_retry_counter);
        assert!(matches!(result, Err(ProvingTaskError::TaskNotFound(_))));
    }

    #[test]
    fn test_dependency_resolution() {
        let mut tracker = TaskTracker::new();
        let (id, deps) = gen_task_with_deps(2);
        let db = setup_db();
        let max_retry_counter = 15u64;

        for dep in &deps {
            tracker.insert_task(*dep, &[], &db).unwrap();
        }
        tracker.insert_task(id, &deps, &db).unwrap();

        for dep in &deps {
            tracker
                .update_status(
                    *dep,
                    ProvingTaskStatus::ProvingInProgress,
                    max_retry_counter,
                )
                .and_then(|_| {
                    tracker.update_status(*dep, ProvingTaskStatus::Completed, max_retry_counter)
                })
                .unwrap();
        }
        assert!(
            matches!(tracker.get_task(id), Ok(&ProvingTaskStatus::Pending)),
            "Task should become Pending after all dependencies are resolved"
        );
    }

    #[test]
    fn test_transient_failures() {
        let mut tracker = TaskTracker::new();
        let (id, _) = gen_task_with_deps(0);
        let db = setup_db();
        let max_retry_counter = 15u64; // Use a specific test value

        // Insert task and mark it as in progress.
        tracker.insert_task(id, &[], &db).unwrap();
        tracker
            .update_status(id, ProvingTaskStatus::ProvingInProgress, max_retry_counter)
            .unwrap();

        // Check transient failures up to max_retry_counter.
        for _ in 0..max_retry_counter {
            tracker
                .update_status(id, ProvingTaskStatus::TransientFailure, max_retry_counter)
                .unwrap();
            assert!(matches!(
                tracker.get_task(id),
                Ok(&ProvingTaskStatus::TransientFailure)
            ));

            // Update back to in progress.
            tracker
                .update_status(id, ProvingTaskStatus::ProvingInProgress, max_retry_counter)
                .unwrap();
        }

        // Check the final transient failure that results in permanent failure.
        tracker
            .update_status(id, ProvingTaskStatus::TransientFailure, max_retry_counter)
            .unwrap();
        assert!(matches!(
            tracker.get_task(id),
            Ok(&ProvingTaskStatus::Failed)
        ));
    }
}
