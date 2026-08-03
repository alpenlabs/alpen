//! Core prover: fetches input via spec, proves via strategy,
//! optionally stores receipt and calls domain hook.

use std::{
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
    error::{FailureAction, ProverError, ProverResult},
    in_memory::InMemoryTaskStore,
    strategy::NativeStrategy,
    task::{now_secs, TaskRecord, TaskResult, TaskStatus},
    traits::{
        InputResolution, ProofSpec, ProveContext, ProveStrategy, ReceiptHook, ReceiptStore,
        TaskStore,
    },
};

/// One completion-notification sender per pending `wait_for_tasks` caller.
///
/// Each waiter receives a private `oneshot::Receiver`; [`Prover::notify`]
/// drains and removes the entry when the task reaches a terminal state.
type WatcherMap<T> = HashMap<Vec<u8>, Vec<oneshot::Sender<TaskResult<T>>>>;

/// Recheck cadence for a blocked task when no retry config is present and the
/// spec gave no per-task override. Real consumers configure
/// `RetryConfig::blocked_recheck_secs`; this is only a floor.
const DEFAULT_BLOCKED_RECHECK_SECS: u64 = 10;

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
        let key: Vec<u8> = task.clone().into();

        // Idempotent: if already in store, skip.
        if self.task_store.get(&key)?.is_some() {
            return Ok(());
        }

        self.task_store
            .insert(TaskRecord::new(key.clone(), TaskStatus::Pending))?;

        let prover = Arc::clone(self);
        tokio::spawn(async move {
            prover.run_task(task, key).await;
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
            if let Some(task) = decode_task_key::<H>(&key) {
                let prover = Arc::clone(self);
                tokio::spawn(async move {
                    prover.run_task(task, key).await;
                });
            }
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
            let Some(task) = decode_task_key::<H>(&key) else {
                continue;
            };

            if let TaskStatus::Proving {
                retry_count,
                resubmit_count,
            } = record.status()
            {
                // A crash mid-prove is resume-class. Route it through
                // `schedule_retry`, which bumps `retry_count`, applies backoff
                // (a *future* `retry_after`), and either parks it as
                // `TransientFailure` or marks `PermanentFailure` when the budget
                // is exhausted.
                //
                // Crucially we do NOT spawn `run_task` here. `schedule_retry`
                // leaves a future `retry_after`, so the `list_retriable(now)`
                // scan later in this same `tick()` won't match it, and the
                // scanner re-spawns it when the backoff elapses. Spawning here
                // *and* leaving it retriable is exactly the double-spawn that a
                // stale (already-elapsed) `retry_after` used to cause.
                warn!(
                    %task,
                    retry_count = retry_count + 1,
                    "task died mid-Proving; scheduling resume-class retry"
                );
                let status = self.schedule_retry(
                    &key,
                    "process died mid-Proving",
                    *retry_count,
                    *resubmit_count,
                    false,
                );
                // If the budget was exhausted, `schedule_retry` marked it
                // `PermanentFailure`; notify waiters so they don't hang.
                self.notify(&key, &task, &status);
                continue;
            }

            // Pending tasks: never scanned by `list_retriable` (not
            // `wants_rescan`), so spawn them directly to resume proving.
            let prover = Arc::clone(self);
            tokio::spawn(async move {
                prover.run_task(task, key).await;
            });
        }
    }

    /// Read the persisted `(retry_count, resubmit_count)` counters for a task.
    ///
    /// Used at the top of [`Self::run_task`] before status is overwritten to
    /// `Proving`. Returns `(0, 0)` for `Pending` or absent records.
    fn read_counts(&self, key: &[u8]) -> (u32, u32) {
        self.task_store
            .get(key)
            .ok()
            .flatten()
            .map_or((0, 0), |r| match r.status() {
                TaskStatus::Proving {
                    retry_count,
                    resubmit_count,
                }
                | TaskStatus::TransientFailure {
                    retry_count,
                    resubmit_count,
                    ..
                } => (*retry_count, *resubmit_count),
                _ => (0, 0),
            })
    }

    /// Read the persisted `Blocked` recheck count for a task, or `0` if it is
    /// not currently blocked. Snapshotted at the top of [`Self::run_task`]
    /// before the `Proving` overwrite discards the `Blocked` status.
    fn read_blocked_rechecks(&self, key: &[u8]) -> u32 {
        self.task_store
            .get(key)
            .ok()
            .flatten()
            .map_or(0, |r| match r.status() {
                TaskStatus::Blocked { recheck_count, .. } => *recheck_count,
                _ => 0,
            })
    }

    async fn run_task(&self, task: H::Task, key: Vec<u8>) {
        let span = info_span!("prove", task = %task);

        async {
            // Stage checkpoint: if proving already succeeded on a prior attempt
            // the receipt is in the receipt store. Skip resolve_input and the
            // (expensive) prove, and re-run only the post-prove hook + finish —
            // so a transient receipt-hook failure never triggers a re-prove.
            // Only applies to provers with a receipt store configured.
            if let Some(receipt_store) = self.receipt_store.as_ref() {
                match receipt_store.get(&key) {
                    Ok(Some(receipt)) => {
                        debug!("receipt already persisted; skipping prove, running hook only");
                        if let Some(hook) = &self.receipt_hook {
                            if let Err(e) = hook.on_receipt(&task, &receipt).await {
                                error!(%e, "receipt hook failed (checkpoint path)");
                                let (retry_count, resubmit_count) = self.read_counts(&key);
                                let status =
                                    self.handle_error(&key, &e, retry_count, resubmit_count);
                                self.notify(&key, &task, &status);
                                return;
                            }
                        }
                        let _ = self.task_store.update_status(&key, TaskStatus::Completed);
                        info!("task completed (from checkpointed receipt)");
                        self.notify(&key, &task, &TaskStatus::Completed);
                        return;
                    }
                    // No checkpoint yet — fall through to a full prove.
                    Ok(None) => {}
                    Err(e) => {
                        // A transient receipt-store read must NOT be swallowed
                        // as "no receipt": that would fall through and re-run
                        // the (possibly remote, expensive) prove after a prior
                        // success. Classify it (Storage -> RetryResume) and
                        // retry the checkpoint instead.
                        error!(%e, "receipt store read failed on checkpoint path");
                        let (retry_count, resubmit_count) = self.read_counts(&key);
                        let status = self.handle_error(&key, &e, retry_count, resubmit_count);
                        self.notify(&key, &task, &status);
                        return;
                    }
                }
            }

            // Snapshot the retry counter from the persisted record BEFORE
            // flipping status to `Proving`. `schedule_retry` cannot read it
            // from the store after the overwrite below, and `recover` needs
            // the count to survive a mid-Proving crash, so persist it inside
            // the `Proving` status itself.
            let (prior_retry_count, prior_resubmit_count) = self.read_counts(&key);
            // Snapshot the Blocked recheck counter too, for the same reason:
            // the `Proving` overwrite below discards the prior `Blocked` status,
            // so carry the count forward across the Blocked → Proving → Blocked
            // loop to keep the `park_blocked` backstop bounded.
            let prior_blocked_rechecks = self.read_blocked_rechecks(&key);

            let _ = self.task_store.update_status(
                &key,
                TaskStatus::Proving {
                    retry_count: prior_retry_count,
                    resubmit_count: prior_resubmit_count,
                },
            );

            // 1. Resolve input: ready, blocked on a dependency, or rejected.
            let input = match self.spec.resolve_input(&task).await {
                Ok(InputResolution::Ready(input)) => input,
                Ok(InputResolution::Blocked {
                    reason,
                    recheck_after,
                }) => {
                    // Not a failure — park and recheck without notifying waiters
                    // or touching the retry budget (but bounded by the backstop).
                    self.park_blocked(&key, &task, reason, recheck_after, prior_blocked_rechecks);
                    return;
                }
                Ok(InputResolution::Rejected { reason }) => {
                    error!(%reason, "input rejected");
                    let status = TaskStatus::PermanentFailure { error: reason };
                    let _ = self.task_store.update_status(&key, status.clone());
                    self.notify(&key, &task, &status);
                    return;
                }
                Err(e) => {
                    error!(%e, "resolve_input failed");
                    let status =
                        self.handle_error(&key, &e, prior_retry_count, prior_resubmit_count);
                    self.notify(&key, &task, &status);
                    return;
                }
            };

            // 2. Prove (blocking — strategy handles native vs remote)
            let saved_metadata = self
                .task_store
                .get(&key)
                .ok()
                .flatten()
                .and_then(|r| r.metadata().map(|m| m.to_vec()));
            let store = self.task_store.clone();
            let persist_key = key.clone();
            let ctx = ProveContext::new(saved_metadata, move |data| {
                let _ = store.set_metadata(&persist_key, data);
            });

            let strategy = self.strategy.clone();
            let prove_result = spawn_blocking(move || strategy.prove(&input, ctx)).await;

            let receipt = match prove_result {
                Ok(Ok(receipt)) => receipt,
                Ok(Err(e)) => {
                    error!(%e, "prove failed");
                    let status =
                        self.handle_error(&key, &e, prior_retry_count, prior_resubmit_count);
                    self.notify(&key, &task, &status);
                    return;
                }
                Err(e) => {
                    error!(%e, "prove task panicked");
                    let status = TaskStatus::PermanentFailure {
                        error: e.to_string(),
                    };
                    let _ = self.task_store.update_status(&key, status.clone());
                    self.notify(&key, &task, &status);
                    return;
                }
            };

            // 3. Store receipt (if configured)
            if let Some(store) = &self.receipt_store {
                if let Err(e) = store.put(&key, &receipt) {
                    error!(%e, "receipt store put failed");
                    let status =
                        self.handle_error(&key, &e, prior_retry_count, prior_resubmit_count);
                    self.notify(&key, &task, &status);
                    return;
                }
            }

            // 4. Domain hook (if configured)
            if let Some(hook) = &self.receipt_hook {
                if let Err(e) = hook.on_receipt(&task, &receipt).await {
                    error!(%e, "receipt hook failed");
                    let status =
                        self.handle_error(&key, &e, prior_retry_count, prior_resubmit_count);
                    self.notify(&key, &task, &status);
                    return;
                }
            }

            // 5. Done
            let _ = self.task_store.update_status(&key, TaskStatus::Completed);
            info!("task completed");
            self.notify(&key, &task, &TaskStatus::Completed);
        }
        .instrument(span)
        .await;
    }

    /// Park a task that is waiting on an input dependency.
    ///
    /// Sets [`TaskStatus::Blocked`] and schedules a steady recheck via
    /// `retry_after`. Does not touch the retry/resubmit counters or notify
    /// waiters — blocking is an expected wait, not a failure, so the scanner
    /// re-spawns it (via [`TaskStatus::wants_rescan`]) when the recheck is due.
    /// `prior_rechecks` is the recheck count carried by the task's previous
    /// `Blocked` status (snapshotted before the `Proving` overwrite in
    /// [`Self::run_task`]), or `0` on the first block. Once the count exceeds
    /// [`RetryConfig::max_blocked_rechecks`] the dependency is treated as never
    /// going to materialize: the task is promoted to `PermanentFailure` and its
    /// waiters are notified, rather than rechecking — and hanging — forever.
    fn park_blocked(
        &self,
        key: &[u8],
        task: &H::Task,
        reason: String,
        recheck_after: Option<Duration>,
        prior_rechecks: u32,
    ) {
        let recheck_count = prior_rechecks.saturating_add(1);

        // Safety backstop: give up on a dependency that never resolves.
        if let Some(max) = self.config.retry.as_ref().map(|c| c.max_blocked_rechecks) {
            if recheck_count > max {
                warn!(
                    reason,
                    recheck_count, max, "blocked dependency unresolved; marking PermanentFailure"
                );
                let status = TaskStatus::PermanentFailure {
                    error: format!("blocked dependency unresolved after {max} rechecks: {reason}"),
                };
                let _ = self.task_store.update_status(key, status.clone());
                self.notify(key, task, &status);
                return;
            }
        }

        let secs = recheck_after.map(|d| d.as_secs()).unwrap_or_else(|| {
            self.config
                .retry
                .as_ref()
                .map_or(DEFAULT_BLOCKED_RECHECK_SECS, |c| c.blocked_recheck_secs)
        });
        debug!(
            reason,
            recheck_secs = secs,
            recheck_count,
            "task blocked on dependency"
        );
        let _ = self.task_store.update_status(
            key,
            TaskStatus::Blocked {
                reason,
                recheck_count,
            },
        );
        let _ = self
            .task_store
            .set_retry_after(key, now_secs().saturating_add(secs));
    }

    /// Persist the outcome of a failed attempt and return the status written,
    /// so the caller can hand it to [`Self::notify`] without a fallible re-read.
    fn handle_error(
        &self,
        key: &[u8],
        err: &ProverError,
        retry_count: u32,
        resubmit_count: u32,
    ) -> TaskStatus {
        match err.action() {
            FailureAction::RetryResume => {
                self.schedule_retry(key, &err.to_string(), retry_count, resubmit_count, false)
            }
            FailureAction::RetryFresh => {
                self.schedule_retry(key, &err.to_string(), retry_count, resubmit_count, true)
            }
            FailureAction::Permanent => {
                let status = TaskStatus::PermanentFailure {
                    error: err.to_string(),
                };
                let _ = self.task_store.update_status(key, status.clone());
                status
            }
        }
    }

    /// Schedule a retry after backoff.
    ///
    /// Resume-class retries (`fresh == false`) re-poll the same request and
    /// draw from `max_retries`. Resubmit-class retries (`fresh == true`) drop
    /// the saved remote metadata so the next attempt submits a fresh request,
    /// and draw from the smaller `max_resubmits` budget since each one re-runs
    /// the whole proof. When the relevant budget is exhausted the task becomes
    /// `PermanentFailure`. Backoff is keyed on whichever counter advanced.
    fn schedule_retry(
        &self,
        key: &[u8],
        msg: &str,
        retry_count: u32,
        resubmit_count: u32,
        fresh: bool,
    ) -> TaskStatus {
        if let Some(ref cfg) = self.config.retry {
            let (new_retry, new_resubmit, within_budget) = if fresh {
                let n = resubmit_count + 1;
                (retry_count, n, cfg.should_resubmit(n))
            } else {
                let n = retry_count + 1;
                (n, resubmit_count, cfg.should_retry(n))
            };

            if within_budget {
                if fresh {
                    // Drop the dead remote ProofId so the next attempt resubmits.
                    let _ = self.task_store.clear_metadata(key);
                }
                let attempt = if fresh { new_resubmit } else { new_retry };
                warn!(
                    retry_count = new_retry,
                    resubmit_count = new_resubmit,
                    fresh,
                    error = %msg,
                    "scheduling retry"
                );
                let status = TaskStatus::TransientFailure {
                    retry_count: new_retry,
                    resubmit_count: new_resubmit,
                    error: msg.to_string(),
                };
                let _ = self.task_store.update_status(key, status.clone());
                let delay_secs = cfg.jittered_delay_secs(attempt, jitter_seed(key, attempt));
                let _ = self
                    .task_store
                    .set_retry_after(key, now_secs().saturating_add(delay_secs));
                return status;
            }
        }

        let status = TaskStatus::PermanentFailure {
            error: format!("retries exhausted: {msg}"),
        };
        let _ = self.task_store.update_status(key, status.clone());
        status
    }

    /// Fan out the terminal result to every pending waiter and remove the
    /// watcher entry so the map does not grow unbounded.
    ///
    /// Takes the freshly-persisted `status` from the caller rather than
    /// re-reading the store: a transient store read here previously turned into
    /// `None` and bailed *without draining the watchers*, permanently losing an
    /// otherwise terminal notification and hanging every waiter forever. Since
    /// every caller has just written the terminal status, the read was both
    /// fallible and redundant.
    ///
    /// The watchers lock is held across the drain to linearize with
    /// [`Self::wait_for_tasks`], which performs its
    /// check-terminal-then-subscribe decision under the same lock. The caller
    /// persists the terminal status *before* invoking this, so a concurrent
    /// subscriber either observes terminal in the store (and returns without
    /// subscribing) or is drained here.
    fn notify(&self, key: &[u8], task: &H::Task, status: &TaskStatus) {
        let Some(result) = terminal_result(task, status) else {
            // Non-terminal (e.g. a scheduled TransientFailure): waiters keep
            // waiting for a later terminal notification.
            return;
        };
        let mut w = self.watchers.lock();
        if let Some(senders) = w.remove(key) {
            for tx in senders {
                let _ = tx.send(result.clone());
            }
        }
    }
}

/// Deterministic per-task backoff seed (FNV-1a over the key, mixed with the
/// attempt count). Used to jitter retry delays so distinct tasks that failed on
/// the same tick spread their wake-ups instead of retrying in lockstep.
fn jitter_seed(key: &[u8], retry_count: u32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in key {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h.wrapping_add(u64::from(retry_count))
}

/// Decode a storage key back into a typed task.
///
/// Logs and returns `None` on decode failure rather than panicking — a
/// corrupt or schema-drifted key should not take down the prover.
fn decode_task_key<H: ProofSpec>(key: &[u8]) -> Option<H::Task> {
    match H::Task::try_from(key.to_vec()) {
        Ok(task) => Some(task),
        Err(_) => {
            warn!(key = ?key, "failed to decode task key, skipping");
            None
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
        // Sanitize on ingestion so an out-of-range operator config (e.g.
        // `jitter_frac > 1.0`) can't reach the scheduler and collapse the
        // backoff to zero. This is the single choke point every config passes
        // through.
        self.retry = Some(config.sanitized());
        self
    }

    /// Build with a native host (blocking `Program::prove` via `spawn_blocking`).
    pub fn native<Host: ZkVmHost + Send + Sync + 'static>(self, host: Host) -> Prover<H> {
        self.build(Arc::new(NativeStrategy::new(host)))
    }

    /// Build with a remote host (`start_proving` + poll on a long-lived runtime).
    #[cfg(feature = "remote")]
    pub fn remote<Host>(self, host: Host) -> Prover<H>
    where
        Host: ZkVmRemoteHost + Send + Sync + 'static,
    {
        use crate::strategy::RemoteStrategy;
        let local = self
            .retry
            .as_ref()
            .map(|r| r.local.clone())
            .unwrap_or_default();
        self.build(Arc::new(RemoteStrategy::new(
            host,
            Duration::from_secs(10),
            local,
        )))
    }

    /// Build with a remote host and custom poll interval.
    #[cfg(feature = "remote")]
    pub fn remote_with_interval<Host>(self, host: Host, poll_interval: Duration) -> Prover<H>
    where
        Host: ZkVmRemoteHost + Send + Sync + 'static,
    {
        use crate::strategy::RemoteStrategy;
        let local = self
            .retry
            .as_ref()
            .map(|r| r.local.clone())
            .unwrap_or_default();
        self.build(Arc::new(RemoteStrategy::new(host, poll_interval, local)))
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
