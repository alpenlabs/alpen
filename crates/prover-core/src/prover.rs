//! Core prover: fetches input via spec, proves via strategy,
//! optionally stores receipt and calls domain hook.

use std::{
    any::type_name,
    collections::HashMap,
    fmt, slice,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::{sync::oneshot, task::spawn_blocking};
use tracing::{debug, error, info, info_span, warn, Instrument};
use zkaleido::ZkVmHost;
#[cfg(feature = "remote")]
use zkaleido::ZkVmRemoteHost;

use crate::{
    config::{ProverConfig, RetryConfig},
    error::{ProverError, ProverResult},
    in_memory::InMemoryTaskStore,
    strategy::NativeStrategy,
    task::{now_secs, TaskRecord, TaskResult, TaskStatus},
    traits::{
        ProofSpec, ProveContext, ProveStrategy, ReceiptHook, ReceiptStore, TaskKey, TaskStore,
    },
};

/// One completion-notification sender per pending `wait_for_tasks` caller.
///
/// Each waiter receives a private `oneshot::Receiver`; [`Prover::notify`]
/// drains and removes the entry when the task reaches a terminal state.
type WatcherMap<T> = HashMap<Vec<u8>, Vec<oneshot::Sender<TaskResult<T>>>>;

/// A task and its canonical storage key.
///
/// The prover needs the typed task for domain logic and the byte key for
/// storage. Keeping both in one value makes the "these match" invariant
/// explicit instead of passing loose `(task, key)` pairs through helpers.
struct RunningTask<T: TaskKey> {
    task: T,
    key: Vec<u8>,
}

impl<T: TaskKey> RunningTask<T> {
    fn new(task: T) -> Self {
        let key = task.clone().into();
        Self { task, key }
    }

    fn from_stored_key(key: Vec<u8>) -> Option<Self> {
        let task = T::try_from(key.clone()).ok()?;
        let derived_key: Vec<u8> = task.clone().into();
        // Round-trip drift means the task type's byte encoding is not
        // canonical — a programming error in the `TaskKey` impl, not a
        // runtime condition.
        debug_assert_eq!(
            derived_key, key,
            "decoded task key did not round-trip through its TaskKey encoding"
        );
        if derived_key != key {
            return None;
        }

        Some(Self {
            task,
            key: derived_key,
        })
    }

    fn task(&self) -> &T {
        &self.task
    }

    fn key(&self) -> &[u8] {
        &self.key
    }
}

/// Single-proof-type prover.
///
/// Generic over `H` (spec) only. The zkVM host type is erased inside
/// the `ProveStrategy` — consumers never see it.
pub struct Prover<H: ProofSpec> {
    spec: Arc<H>,
    strategy: Arc<dyn ProveStrategy<H>>,
    config: ProverConfig,
    task_store: Arc<dyn TaskStore>,
    receipt_store: Option<Arc<dyn ReceiptStore>>,
    receipt_hook: Option<Arc<dyn ReceiptHook<H>>>,
    /// Oneshot senders for notifying waiters when tasks reach terminal states.
    watchers: Arc<Mutex<WatcherMap<H::Task>>>,
    /// Whether we've run recovery on startup.
    recovered: AtomicBool,
}

impl<H: ProofSpec> fmt::Debug for Prover<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prover")
            .field("has_retry", &self.config.retry.is_some())
            .field("has_receipt_store", &self.receipt_store.is_some())
            .field("has_receipt_hook", &self.receipt_hook.is_some())
            .finish()
    }
}

// Prover is never cloned directly. Spawning methods take `self: &Arc<Self>`
// so background tasks hold a cheap Arc refcount instead of shallow-cloning
// every field. External consumers go through ProverHandle, which already
// stores an `Arc<Prover>`.

// ============================================================================
// Consumer API
// ============================================================================

impl<H: ProofSpec> Prover<H> {
    /// Register a task and spawn background proving. Idempotent.
    pub async fn submit(self: &Arc<Self>, task: H::Task) -> ProverResult<()> {
        let task = RunningTask::new(task);

        // Idempotent: if already in store, skip.
        if self.task_store.get(task.key())?.is_some() {
            return Ok(());
        }

        if let Err(e) = self
            .task_store
            .insert(TaskRecord::new(task.key.clone(), TaskStatus::Pending))
        {
            if matches!(e, ProverError::TaskAlreadyExists(_)) {
                return Ok(());
            }
            return Err(e);
        }

        let prover = Arc::clone(self);
        tokio::spawn(async move {
            prover.run_task(task).await;
        });

        Ok(())
    }

    /// Submit a task and block until it reaches a terminal state.
    pub async fn execute(self: &Arc<Self>, task: H::Task) -> ProverResult<TaskResult<H::Task>> {
        self.submit(task.clone()).await?;
        let results = self.wait_for_tasks(slice::from_ref(&task)).await?;
        Ok(results.into_iter().next().expect("one result for one task"))
    }

    /// Block until all tasks reach terminal states.
    ///
    /// Zero polling: each waiter receives a private `oneshot` receiver that
    /// fires exactly once when the task reaches a terminal state. The
    /// subscribe-or-observe-completion step is linearized against
    /// `Self::notify` via the watchers mutex, so the wait cannot miss
    /// completions that race with subscription.
    pub async fn wait_for_tasks(
        &self,
        tasks: &[H::Task],
    ) -> ProverResult<Vec<TaskResult<H::Task>>> {
        let mut results: Vec<Option<TaskResult<H::Task>>> = vec![None; tasks.len()];
        let mut pending: Vec<(usize, oneshot::Receiver<TaskResult<H::Task>>)> = Vec::new();

        for (i, task) in tasks.iter().enumerate() {
            let key: Vec<u8> = task.clone().into();

            // Hold the watchers lock across the store check + subscribe so
            // we cannot miss a notification that races with this decision.
            let mut w = self.watchers.lock();
            if let Some(record) = self.task_store.get(&key)? {
                if let Some(r) = terminal_result(task, record.status()) {
                    results[i] = Some(r);
                    continue;
                }
            }
            let (tx, rx) = oneshot::channel();
            w.entry(key).or_default().push(tx);
            drop(w);

            pending.push((i, rx));
        }

        for (i, rx) in pending {
            // `rx.await` can only fail if the sender was dropped without
            // sending — we never do that: `notify` drains the entry on
            // completion, and the entry is only created here. Treat a dropped
            // sender as a permanent-failure signal rather than panicking.
            match rx.await {
                Ok(result) => results[i] = Some(result),
                Err(_) => {
                    results[i] = Some(TaskResult::failed(
                        tasks[i].clone(),
                        "notification sender dropped".to_string(),
                    ));
                }
            }
        }

        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    }

    /// Remove a terminal task record so the task can be resubmitted from
    /// scratch.
    ///
    /// Returns `true` if the record was removed (or was already absent).
    /// A non-terminal record is left in place and `false` is returned:
    /// the in-flight attempt still owns it and will drive it to a terminal
    /// state on its own.
    ///
    /// The check-then-remove is not atomic: two concurrent resetters can
    /// interleave with a resubmit so that a freshly re-inserted record is
    /// removed. That is self-healing — the next status poll sees the task
    /// as absent and resubmits — costing at most a wasted proving run.
    pub fn reset_task(&self, task: &H::Task) -> ProverResult<bool> {
        let key: Vec<u8> = task.clone().into();
        match self.task_store.get(&key)? {
            None => Ok(true),
            Some(record) => match record.status() {
                TaskStatus::Completed | TaskStatus::PermanentFailure { .. } => {
                    self.task_store.remove(&key)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
        }
    }

    /// Get a receipt from the receipt store by task.
    ///
    /// Returns `None` if the store has no receipt for this task, or `Err` if
    /// no receipt store was configured.
    pub fn get_receipt(
        &self,
        task: &H::Task,
    ) -> ProverResult<Option<zkaleido::ProofReceiptWithMetadata>> {
        let key: Vec<u8> = task.clone().into();
        self.receipt_store
            .as_ref()
            .ok_or(ProverError::NoReceiptStore)?
            .get(&key)
    }
}

// ============================================================================
// Internals - PaaS wiring + proving flow (not exposed on ProverHandle)
// ============================================================================

impl<H: ProofSpec> Prover<H> {
    pub fn has_retry(&self) -> bool {
        self.config.retry.is_some()
    }

    pub fn has_receipt_store(&self) -> bool {
        self.receipt_store.is_some()
    }

    pub fn task_store(&self) -> &dyn TaskStore {
        self.task_store.as_ref()
    }

    /// Current task status by task.
    pub fn get_status(&self, task: &H::Task) -> ProverResult<TaskStatus> {
        let key: Vec<u8> = task.clone().into();
        self.task_store
            .get(&key)?
            .map(|r| r.status().clone())
            .ok_or_else(|| ProverError::TaskNotFound(format!("{task}")))
    }

    /// Scan for retriable tasks and re-spawn them. Called by PaaS on tick.
    pub async fn tick(self: &Arc<Self>) {
        if !self.recovered.swap(true, Ordering::SeqCst) {
            self.recover().await;
        }

        let retriable = match self.task_store.list_retriable(now_secs()) {
            Ok(v) => v,
            Err(e) => {
                warn!(%e, "failed to list retriable tasks");
                return;
            }
        };
        for record in retriable {
            let key = record.key().to_vec();
            let Some(task) = RunningTask::from_stored_key(key.clone()) else {
                warn!(?key, "skipping retriable task with undecodable key");
                continue;
            };
            let prover = Arc::clone(self);
            tokio::spawn(async move {
                prover.run_task(task).await;
            });
        }
    }

    /// Re-spawn every unfinished task on startup — anything not yet terminal
    /// (Pending or Proving). Before this change we only re-picked in-progress
    /// work, so a crash between `submit`'s db insert and the spawn would
    /// leave a task stuck in Pending forever.
    ///
    /// A task found in `Proving` is one whose previous attempt died
    /// abnormally — the process was killed (OOM, SIGKILL, panic) before any
    /// error path could run. In that case no `schedule_retry` ever happened,
    /// so the retry counter would otherwise stay at its pre-attempt value
    /// forever and the same crash-inducing task would re-run indefinitely.
    /// To bound this, recovery treats the dead attempt as a synthetic
    /// transient failure: bump the counter and either schedule a normal
    /// retry or, if `max_retries` is exhausted, mark `PermanentFailure` and
    /// skip the spawn.
    async fn recover(self: &Arc<Self>) {
        let unfinished = match self.task_store.list_unfinished() {
            Ok(v) => v,
            Err(e) => {
                warn!(%e, "failed to list unfinished tasks during recovery");
                return;
            }
        };
        if unfinished.is_empty() {
            return;
        }
        info!(count = unfinished.len(), "recovering unfinished tasks");
        for record in unfinished {
            let key = record.key().to_vec();
            let Some(task) = RunningTask::from_stored_key(key.clone()) else {
                warn!(?key, "skipping unfinished task with undecodable key");
                continue;
            };

            if let TaskStatus::Proving { retry_count } = record.status() {
                let new_count = retry_count + 1;
                let should_retry = self
                    .config
                    .retry
                    .as_ref()
                    .is_some_and(|cfg| cfg.should_retry(new_count));

                if !should_retry {
                    self.mark_permanent_failure(
                        &task,
                        format!("process died mid-Proving; retries exhausted at {new_count}"),
                    );
                    self.notify(&task);
                    continue;
                }

                warn!(
                    task = %task.task(),
                    retry_count = new_count,
                    "task died mid-Proving; counting as transient failure"
                );
                let _ = self.task_store.update_status(
                    task.key(),
                    TaskStatus::TransientFailure {
                        retry_count: new_count,
                        error: "process died mid-Proving".to_string(),
                    },
                );
                // Fall through to spawn — `run_task` will snapshot the bumped
                // count from the now-TransientFailure record.
            }

            let prover = Arc::clone(self);
            tokio::spawn(async move {
                prover.run_task(task).await;
            });
        }
    }

    /// Read the persisted retry counter for a task.
    ///
    /// Used at the top of [`Self::run_task`] before status is overwritten to
    /// `Proving`, and by [`Self::recover`] to compute the post-crash bump.
    /// Returns 0 for `Pending` or absent records.
    fn read_retry_count(&self, task: &RunningTask<H::Task>) -> u32 {
        self.task_store
            .get(task.key())
            .ok()
            .flatten()
            .map_or(0, |r| match r.status() {
                TaskStatus::Proving { retry_count }
                | TaskStatus::TransientFailure { retry_count, .. } => *retry_count,
                _ => 0,
            })
    }

    async fn run_task(&self, task: RunningTask<H::Task>) {
        let span = info_span!("prove", task = %task.task());

        async {
            // Snapshot the retry counter from the persisted record BEFORE
            // flipping status to `Proving`. `schedule_retry` cannot read it
            // from the store after the overwrite below, and `recover` needs
            // the count to survive a mid-Proving crash, so persist it inside
            // the `Proving` status itself.
            let prior_retry_count = self.read_retry_count(&task);

            let _ = self.task_store.update_status(
                task.key(),
                TaskStatus::Proving {
                    retry_count: prior_retry_count,
                },
            );

            // 1. Fetch input
            let input = match self.spec.fetch_input(task.task()).await {
                Ok(input) => input,
                Err(e) => {
                    if e.is_transient() {
                        warn!(%e, "fetch_input transient failure");
                    }
                    self.handle_error(&task, &e, prior_retry_count);
                    self.notify(&task);
                    return;
                }
            };

            // 2. Prove (blocking — strategy handles native vs remote)
            let saved_metadata = self
                .task_store
                .get(task.key())
                .ok()
                .flatten()
                .and_then(|r| r.metadata().map(|m| m.to_vec()));
            let store = self.task_store.clone();
            let persist_key = task.key.clone();
            let ctx = ProveContext::new(saved_metadata, move |data| {
                let _ = store.set_metadata(&persist_key, data);
            });

            let strategy = self.strategy.clone();
            let prove_result = spawn_blocking(move || strategy.prove(&input, ctx)).await;

            let receipt = match prove_result {
                Ok(Ok(receipt)) => receipt,
                Ok(Err(e)) => {
                    if e.is_transient() {
                        warn!(%e, "prove transient failure");
                    }
                    self.handle_error(&task, &e, prior_retry_count);
                    self.notify(&task);
                    return;
                }
                Err(e) => {
                    self.mark_permanent_failure(&task, format!("prove task panicked: {e}"));
                    self.notify(&task);
                    return;
                }
            };

            // 3. Store receipt (if configured)
            if let Some(store) = &self.receipt_store {
                if let Err(e) = store.put(task.key(), &receipt) {
                    if e.is_transient() {
                        warn!(%e, "receipt store put transient failure");
                    }
                    self.handle_error(&task, &e, prior_retry_count);
                    self.notify(&task);
                    return;
                }
            }

            // 4. Domain hook (if configured)
            if let Some(hook) = &self.receipt_hook {
                if let Err(e) = hook.on_receipt(task.task(), &receipt).await {
                    if e.is_transient() {
                        warn!(%e, "receipt hook transient failure");
                    }
                    self.handle_error(&task, &e, prior_retry_count);
                    self.notify(&task);
                    return;
                }
            }

            // 5. Done
            let _ = self
                .task_store
                .update_status(task.key(), TaskStatus::Completed);
            info!("task completed");
            self.notify(&task);
        }
        .instrument(span)
        .await;
    }

    fn handle_error(&self, task: &RunningTask<H::Task>, err: &ProverError, prior_retry_count: u32) {
        if err.is_transient() {
            self.schedule_retry(task, &err.to_string(), prior_retry_count);
        } else {
            self.mark_permanent_failure(task, err.to_string());
        }
    }

    fn schedule_retry(&self, task: &RunningTask<H::Task>, msg: &str, prior_retry_count: u32) {
        let new_count = prior_retry_count + 1;

        if let Some(ref cfg) = self.config.retry {
            if cfg.should_retry(new_count) {
                warn!(
                    retry_count = new_count,
                    error = %msg,
                    "transient failure, scheduling retry"
                );
                let _ = self.task_store.update_status(
                    task.key(),
                    TaskStatus::TransientFailure {
                        retry_count: new_count,
                        error: msg.to_string(),
                    },
                );
                let delay = Duration::from_secs(cfg.calculate_delay(new_count));
                let _ = self
                    .task_store
                    .set_retry_after(task.key(), now_secs() + delay.as_secs());
                return;
            }
        }

        self.mark_permanent_failure(task, format!("retries exhausted: {msg}"));
    }

    #[tracing::instrument(skip_all, fields(
        proof_spec = type_name::<H>(),
        task_type = type_name::<H::Task>(),
        program_type = type_name::<H::Program>(),
        task = %task.task(),
    ))]
    fn mark_permanent_failure(&self, task: &RunningTask<H::Task>, error: String) {
        let previous_status = self
            .task_store
            .get(task.key())
            .ok()
            .flatten()
            .map(|record| record.status().clone());

        let _ = self.task_store.update_status(
            task.key(),
            TaskStatus::PermanentFailure {
                error: error.clone(),
            },
        );

        if should_log_permanent_failure(previous_status.as_ref(), &error) {
            error!(
                reason = %error,
                "CRITICAL: proof task permanently failed; manual intervention may be required"
            );
        } else {
            debug!(reason = %error, "proof task remains permanently failed");
        }
    }

    /// Fan out the terminal result to every pending waiter and remove the
    /// watcher entry so the map does not grow unbounded.
    ///
    /// The watchers lock is held across the store read to linearize with
    /// [`Self::wait_for_tasks`], which performs its
    /// check-terminal-then-subscribe decision under the same lock.
    fn notify(&self, task: &RunningTask<H::Task>) {
        let mut w = self.watchers.lock();
        let status = self
            .task_store
            .get(task.key())
            .ok()
            .flatten()
            .map(|r| r.status().clone());
        let Some(result) = status
            .as_ref()
            .and_then(|s| terminal_result(task.task(), s))
        else {
            return;
        };
        if let Some(senders) = w.remove(task.key()) {
            for tx in senders {
                let _ = tx.send(result.clone());
            }
        }
    }
}

/// Map a task status to a terminal [`TaskResult`] if it represents one.
fn terminal_result<T: Clone>(task: &T, status: &TaskStatus) -> Option<TaskResult<T>> {
    match status {
        TaskStatus::Completed => Some(TaskResult::completed(task.clone())),
        TaskStatus::PermanentFailure { error } => {
            Some(TaskResult::failed(task.clone(), error.clone()))
        }
        _ => None,
    }
}

fn should_log_permanent_failure(previous: Option<&TaskStatus>, error: &str) -> bool {
    !matches!(previous, Some(TaskStatus::PermanentFailure { error: previous }) if previous == error)
}

// ============================================================================
// Builder
// ============================================================================

/// Builds a [`Prover`].
pub struct ProverBuilder<H: ProofSpec> {
    spec: H,
    task_store: Option<Arc<dyn TaskStore>>,
    receipt_store: Option<Arc<dyn ReceiptStore>>,
    receipt_hook: Option<Arc<dyn ReceiptHook<H>>>,
    retry: Option<RetryConfig>,
}

impl<H: ProofSpec> ProverBuilder<H> {
    pub fn new(spec: H) -> Self {
        Self {
            spec,
            task_store: None,
            receipt_store: None,
            receipt_hook: None,
            retry: None,
        }
    }

    pub fn task_store(mut self, store: impl TaskStore + 'static) -> Self {
        self.task_store = Some(Arc::new(store));
        self
    }

    /// Opt-in receipt persistence. Enables `get_receipt` on the PaaS handle.
    pub fn receipt_store(mut self, store: impl ReceiptStore + 'static) -> Self {
        self.receipt_store = Some(Arc::new(store));
        self
    }

    /// Opt-in domain hook called after receipt storage.
    pub fn receipt_hook(mut self, hook: impl ReceiptHook<H> + 'static) -> Self {
        self.receipt_hook = Some(Arc::new(hook));
        self
    }

    pub fn retry(mut self, config: RetryConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Build with a native host (blocking `Program::prove` via `spawn_blocking`).
    pub fn native<Host: ZkVmHost + Send + Sync + 'static>(self, host: Host) -> Prover<H> {
        self.build(Arc::new(NativeStrategy::new(host)))
    }

    /// Build with a remote host (`start_proving` + poll via `LocalSet`).
    #[cfg(feature = "remote")]
    pub fn remote<Host>(self, host: Host) -> Prover<H>
    where
        Host: ZkVmRemoteHost + Send + Sync + 'static,
    {
        use crate::strategy::RemoteStrategy;
        self.build(Arc::new(RemoteStrategy::new(host, Duration::from_secs(10))))
    }

    /// Build with a remote host and custom poll interval.
    #[cfg(feature = "remote")]
    pub fn remote_with_interval<Host>(self, host: Host, poll_interval: Duration) -> Prover<H>
    where
        Host: ZkVmRemoteHost + Send + Sync + 'static,
    {
        use crate::strategy::RemoteStrategy;
        self.build(Arc::new(RemoteStrategy::new(host, poll_interval)))
    }

    fn build(self, strategy: Arc<dyn ProveStrategy<H>>) -> Prover<H> {
        Prover {
            spec: Arc::new(self.spec),
            strategy,
            config: ProverConfig { retry: self.retry },
            task_store: self
                .task_store
                .unwrap_or_else(|| Arc::new(InMemoryTaskStore::new())),
            receipt_store: self.receipt_store,
            receipt_hook: self.receipt_hook,
            watchers: Arc::new(Mutex::new(HashMap::new())),
            recovered: AtomicBool::new(false),
        }
    }
}

impl<H: ProofSpec> fmt::Debug for ProverBuilder<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverBuilder").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_failure_logging_detects_transitions() {
        assert!(should_log_permanent_failure(None, "bad witness"));
        assert!(should_log_permanent_failure(
            Some(&TaskStatus::Pending),
            "bad witness"
        ));
        assert!(!should_log_permanent_failure(
            Some(&TaskStatus::PermanentFailure {
                error: "bad witness".to_string(),
            }),
            "bad witness"
        ));
        assert!(should_log_permanent_failure(
            Some(&TaskStatus::PermanentFailure {
                error: "old reason".to_string(),
            }),
            "bad witness"
        ));
    }
}
