//! Core prover: fetches input via spec, proves via strategy,
//! optionally stores receipt and calls domain hook.

use std::{
    collections::HashMap,
    fmt, slice,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use tokio::sync::{watch, RwLock};
use tracing::{error, info, info_span, warn, Instrument};
use zkaleido::ZkVmHost;
#[cfg(feature = "remote")]
use zkaleido::ZkVmRemoteHost;

use crate::{
    config::{ProverConfig, RetryConfig},
    error::{ProverError, ProverResult},
    receipt::{ReceiptHook, ReceiptStore},
    spec::ProofSpec,
    store::{InMemoryTaskStore, TaskRecord, TaskStore},
    strategy::{NativeStrategy, ProveContext, ProveStrategy},
    task::{TaskResult, TaskStatus},
};

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
    /// Watch channels for notifying waiters when tasks reach terminal states.
    watchers: Arc<RwLock<HashMap<Vec<u8>, watch::Sender<Option<TaskResult<H::Task>>>>>>,
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

impl<H: ProofSpec> Clone for Prover<H> {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            strategy: self.strategy.clone(),
            config: self.config.clone(),
            task_store: self.task_store.clone(),
            receipt_store: self.receipt_store.clone(),
            receipt_hook: self.receipt_hook.clone(),
            watchers: self.watchers.clone(),
            recovered: AtomicBool::new(self.recovered.load(Ordering::SeqCst)),
        }
    }
}

// ============================================================================
// Consumer API
// ============================================================================

impl<H: ProofSpec> Prover<H> {
    /// Register a task and spawn background proving. Idempotent.
    pub async fn submit(&self, task: H::Task) -> ProverResult<()> {
        let key: Vec<u8> = task.clone().into();

        // Idempotent: if already in store, skip.
        if self.task_store.get(&key).is_some() {
            return Ok(());
        }

        self.task_store
            .insert(TaskRecord::new(key.clone(), TaskStatus::Pending))?;

        let prover = self.clone();
        tokio::spawn(async move {
            prover.run_task(task, key).await;
        });

        Ok(())
    }

    /// Submit a task and block until it reaches a terminal state.
    pub async fn execute(&self, task: H::Task) -> ProverResult<TaskResult<H::Task>> {
        self.submit(task.clone()).await?;
        let results = self.wait_for_tasks(slice::from_ref(&task)).await?;
        Ok(results.into_iter().next().expect("one result for one task"))
    }

    /// Block until all tasks reach terminal states.
    ///
    /// Uses watch channels — zero polling, immediate notification.
    pub async fn wait_for_tasks(
        &self,
        tasks: &[H::Task],
    ) -> ProverResult<Vec<TaskResult<H::Task>>> {
        let mut receivers: Vec<(usize, watch::Receiver<Option<TaskResult<H::Task>>>)> = Vec::new();
        let mut results: Vec<Option<TaskResult<H::Task>>> = vec![None; tasks.len()];

        for (i, task) in tasks.iter().enumerate() {
            let key: Vec<u8> = task.clone().into();
            if let Some(record) = self.task_store.get(&key) {
                match record.status() {
                    TaskStatus::Completed => {
                        results[i] = Some(TaskResult::completed(task.clone()));
                        continue;
                    }
                    TaskStatus::PermanentFailure { error } => {
                        results[i] = Some(TaskResult::failed(task.clone(), error));
                        continue;
                    }
                    _ => {}
                }
            }

            let rx = {
                let mut w = self.watchers.write().await;
                if let Some(tx) = w.get(&key) {
                    tx.subscribe()
                } else {
                    let (tx, rx) = watch::channel(None);
                    w.insert(key, tx);
                    rx
                }
            };
            receivers.push((i, rx));
        }

        if receivers.is_empty() {
            return Ok(results.into_iter().map(|r| r.unwrap()).collect());
        }

        loop {
            for (i, rx) in &receivers {
                if results[*i].is_some() {
                    continue;
                }
                if let Some(result) = rx.borrow().as_ref() {
                    results[*i] = Some(result.clone());
                }
            }

            if results.iter().all(|r| r.is_some()) {
                return Ok(results.into_iter().map(|r| r.unwrap()).collect());
            }

            let futs: Vec<_> = receivers
                .iter()
                .filter(|(i, _)| results[*i].is_none())
                .map(|(_, rx)| {
                    let mut rx = rx.clone();
                    Box::pin(async move { rx.changed().await })
                })
                .collect();
            use futures::future::select_all;
            let _ = select_all(futs).await;
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
            .ok_or_else(|| ProverError::Internal(anyhow::anyhow!("no receipt store configured")))?
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
            .get(&key)
            .map(|r| r.status().clone())
            .ok_or_else(|| ProverError::TaskNotFound(format!("{task}")))
    }

    /// Scan for retriable tasks and re-spawn them. Called by PaaS on tick.
    pub async fn tick(&self) {
        if !self.recovered.swap(true, Ordering::SeqCst) {
            self.recover().await;
        }

        for record in self.task_store.list_retriable(SystemTime::now()) {
            let key = record.key().to_vec();
            if let Some(task) = self.task_from_key(&key) {
                let prover = self.clone();
                tokio::spawn(async move {
                    prover.run_task(task, key).await;
                });
            }
        }
    }

    async fn recover(&self) {
        let in_progress = self.task_store.list_in_progress();
        if in_progress.is_empty() {
            return;
        }
        info!(count = in_progress.len(), "recovering in-progress tasks");
        for record in in_progress {
            let key = record.key().to_vec();
            if let Some(task) = self.task_from_key(&key) {
                let prover = self.clone();
                tokio::spawn(async move {
                    prover.run_task(task, key).await;
                });
            }
        }
    }

    /// Deserialize a task from its storage key bytes.
    fn task_from_key(&self, key: &[u8]) -> Option<H::Task> {
        match H::Task::try_from(key.to_vec()) {
            Ok(task) => Some(task),
            Err(_) => {
                warn!(key = ?key, "failed to deserialize task from key, skipping");
                None
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
                    self.notify(&key, &task).await;
                    return;
                }
            };

            // 2. Prove (blocking — strategy handles native vs remote)
            let saved_metadata = self
                .task_store
                .get(&key)
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
                    self.notify(&key, &task).await;
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
                    self.notify(&key, &task).await;
                    return;
                }
            };

            // 3. Store receipt (if configured)
            if let Some(store) = &self.receipt_store {
                if let Err(e) = store.put(&key, &receipt) {
                    error!(%e, "receipt store put failed");
                    self.handle_error(&key, &e);
                    self.notify(&key, &task).await;
                    return;
                }
            }

            // 4. Domain hook (if configured)
            if let Some(hook) = &self.receipt_hook {
                if let Err(e) = hook.on_receipt(&task, &receipt).await {
                    error!(%e, "receipt hook failed");
                    self.handle_error(&key, &e);
                    self.notify(&key, &task).await;
                    return;
                }
            }

            // 5. Done
            let _ = self.task_store.update_status(&key, TaskStatus::Completed);
            info!("task completed");
            self.notify(&key, &task).await;
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
            .and_then(|r| match r.status() {
                TaskStatus::TransientFailure { retry_count, .. } => Some(*retry_count),
                _ => None,
            })
            .unwrap_or(0);
        let new_count = current_count + 1;

        if let Some(ref cfg) = self.config.retry {
            if cfg.should_retry(new_count) {
                warn!(retry_count = new_count, "transient failure, scheduling retry");
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
                    .set_retry_after(key, SystemTime::now() + delay);
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

    async fn notify(&self, key: &[u8], task: &H::Task) {
        let result = self
            .task_store
            .get(key)
            .and_then(|r| match r.status() {
                TaskStatus::Completed => Some(TaskResult::completed(task.clone())),
                TaskStatus::PermanentFailure { error } => {
                    Some(TaskResult::failed(task.clone(), error))
                }
                _ => None,
            });

        if let Some(result) = result {
            if let Some(tx) = self.watchers.read().await.get(key) {
                let _ = tx.send(Some(result));
            }
        }
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
            watchers: Arc::new(RwLock::new(HashMap::new())),
            recovered: AtomicBool::new(false),
        }
    }
}

impl<H: ProofSpec> fmt::Debug for ProverBuilder<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverBuilder").finish()
    }
}
