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
use tokio::sync::oneshot;
use tracing::{error, info, info_span, warn, Instrument};
use zkaleido::ZkVmHost;
#[cfg(feature = "remote")]
use zkaleido::ZkVmRemoteHost;

use crate::{
    config::{ProverConfig, RetryConfig},
    error::{ProverError, ProverResult},
    receipt::{ReceiptHook, ReceiptStore},
    spec::ProofSpec,
    store::{now_secs, InMemoryTaskStore, TaskRecord, TaskStore},
    strategy::{NativeStrategy, ProveContext, ProveStrategy},
    task::{TaskResult, TaskStatus},
};

/// One completion-notification sender per pending `wait_for_tasks` caller.
///
/// Each waiter receives a private `oneshot::Receiver`; [`Prover::notify`]
/// drains and removes the entry when the task reaches a terminal state.
type WatcherMap<T> = HashMap<Vec<u8>, Vec<oneshot::Sender<TaskResult<T>>>>;

/// Single-proof-type prover.
///
/// Generic over `H` (spec) only. The zkVM host type is erased inside
/// the [`ProveStrategy`] — consumers never see it.
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
    /// [`Self::notify`] via the watchers mutex, so the wait cannot miss
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
// Internal API (used by PaaS tick, not exposed on ProverHandle)
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
    /// (Pending, Queued, or Proving). Before this change we only re-picked
    /// in-progress work, so a crash between `submit`'s db insert and the
    /// spawn would leave a task stuck in Pending forever.
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
            if let Some(task) = decode_task_key::<H>(&key) {
                let prover = Arc::clone(self);
                tokio::spawn(async move {
                    prover.run_task(task, key).await;
                });
            }
        }
    }
}

// ============================================================================
// Proving internals
// ============================================================================

impl<H: ProofSpec> Prover<H> {
    async fn run_task(&self, task: H::Task, key: Vec<u8>) {
        use tokio::task::spawn_blocking;

        let span = info_span!("prove", task = %task);

        async {
            let _ = self.task_store.update_status(&key, TaskStatus::Proving);

            // 1. Fetch input
            let input = match self.spec.fetch_input(&task).await {
                Ok(input) => input,
                Err(e) => {
                    self.handle_error(&key, &e);
                    self.notify(&key, &task);
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
                    self.handle_error(&key, &e);
                    self.notify(&key, &task);
                    return;
                }
                Err(e) => {
                    error!(%e, "prove task panicked");
                    let _ = self.task_store.update_status(
                        &key,
                        TaskStatus::PermanentFailure {
                            error: e.to_string(),
                        },
                    );
                    self.notify(&key, &task);
                    return;
                }
            };

            // 3. Store receipt (if configured)
            if let Some(store) = &self.receipt_store {
                if let Err(e) = store.put(&key, &receipt) {
                    error!(%e, "receipt store put failed");
                    self.handle_error(&key, &e);
                    self.notify(&key, &task);
                    return;
                }
            }

            // 4. Domain hook (if configured)
            if let Some(hook) = &self.receipt_hook {
                if let Err(e) = hook.on_receipt(&task, &receipt).await {
                    error!(%e, "receipt hook failed");
                    self.handle_error(&key, &e);
                    self.notify(&key, &task);
                    return;
                }
            }

            // 5. Done
            let _ = self.task_store.update_status(&key, TaskStatus::Completed);
            info!("task completed");
            self.notify(&key, &task);
        }
        .instrument(span)
        .await;
    }

    fn handle_error(&self, key: &[u8], err: &ProverError) {
        if err.is_transient() {
            self.schedule_retry(key, &err.to_string());
        } else {
            let _ = self.task_store.update_status(
                key,
                TaskStatus::PermanentFailure {
                    error: err.to_string(),
                },
            );
        }
    }

    fn schedule_retry(&self, key: &[u8], msg: &str) {
        let current_count = self
            .task_store
            .get(key)
            .ok()
            .flatten()
            .and_then(|r| match r.status() {
                TaskStatus::TransientFailure { retry_count, .. } => Some(*retry_count),
                _ => None,
            })
            .unwrap_or(0);
        let new_count = current_count + 1;

        if let Some(ref cfg) = self.config.retry {
            if cfg.should_retry(new_count) {
                warn!(
                    retry_count = new_count,
                    "transient failure, scheduling retry"
                );
                let _ = self.task_store.update_status(
                    key,
                    TaskStatus::TransientFailure {
                        retry_count: new_count,
                        error: msg.to_string(),
                    },
                );
                let delay = Duration::from_secs(cfg.calculate_delay(new_count));
                let _ = self
                    .task_store
                    .set_retry_after(key, now_secs() + delay.as_secs());
                return;
            }
        }

        let _ = self.task_store.update_status(
            key,
            TaskStatus::PermanentFailure {
                error: format!("retries exhausted: {msg}"),
            },
        );
    }

    /// Fan out the terminal result to every pending waiter and remove the
    /// watcher entry so the map does not grow unbounded.
    ///
    /// The watchers lock is held across the store read to linearize with
    /// [`Self::wait_for_tasks`], which performs its
    /// check-terminal-then-subscribe decision under the same lock.
    fn notify(&self, key: &[u8], task: &H::Task) {
        let mut w = self.watchers.lock();
        let status = self
            .task_store
            .get(key)
            .ok()
            .flatten()
            .map(|r| r.status().clone());
        let Some(result) = status.as_ref().and_then(|s| terminal_result(task, s)) else {
            return;
        };
        if let Some(senders) = w.remove(key) {
            for tx in senders {
                let _ = tx.send(result.clone());
            }
        }
    }
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
