//! Builder for direct handler-based prover service

use std::{collections::HashMap, fmt, sync::Arc};

use strata_service::ServiceBuilder;
use strata_tasks::TaskExecutor;
use tokio::sync::{mpsc, Semaphore};

use crate::{
    config::{ProverServiceConfig, RetryConfig},
    error::{ProverServiceError, ProverServiceResult},
    handler::ProofHandler,
    persistence::TaskStore,
    program::ProgramType,
    scheduler::{RetryScheduler, SchedulerCommand, SchedulerHandle},
    service::{handle::ProverHandle, runtime::ProverService, state::ProverServiceState},
    ZkVmBackend,
};

/// Builder for creating a prover service with direct handler execution
///
/// This builder allows you to register handlers for each program variant,
/// configure semaphores for backend capacity control, and launch the service.
///
/// ## Example
///
/// ```rust,ignore
/// let handle = ProverServiceBuilder::new(config)
///     .with_task_store(task_store)
///     .with_handler(ProofContextVariant::Checkpoint, checkpoint_handler)
///     .with_handler(ProofContextVariant::ClStf, cl_stf_handler)
///     .with_handler(ProofContextVariant::EvmEeStf, evm_ee_handler)
///     .launch(&executor)
///     .await?;
/// ```
pub struct ProverServiceBuilder<P: ProgramType> {
    config: ProverServiceConfig<ZkVmBackend>,
    handlers: HashMap<P::RoutingKey, Arc<dyn ProofHandler<P>>>,
    task_store: Option<Arc<dyn TaskStore<P>>>,
}

impl<P: ProgramType> fmt::Debug for ProverServiceBuilder<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProverServiceBuilder")
            .field("config", &self.config)
            .field("handler_count", &self.handlers.len())
            .field("has_task_store", &self.task_store.is_some())
            .finish()
    }
}

impl<P: ProgramType> ProverServiceBuilder<P> {
    /// Create a new builder with the given configuration
    pub fn new(config: ProverServiceConfig<ZkVmBackend>) -> Self {
        Self {
            config,
            handlers: HashMap::new(),
            task_store: None,
        }
    }

    /// Set the task storage backend for persistent task tracking
    ///
    /// This method configures the `TaskStore` used for persisting
    /// TaskId -> UUID mappings and task status. This is required
    /// for production deployments.
    pub fn with_task_store<S>(mut self, store: S) -> Self
    where
        S: TaskStore<P> + 'static,
    {
        self.task_store = Some(Arc::new(store));
        self
    }

    /// Register a handler for a specific program variant
    ///
    /// Each program variant identified by its routing key needs a handler.
    /// The handler encapsulates all execution complexity (fetch, prove, store).
    pub fn with_handler(mut self, key: P::RoutingKey, handler: Arc<dyn ProofHandler<P>>) -> Self {
        self.handlers.insert(key, handler);
        self
    }

    /// Enable retries with custom configuration
    ///
    /// By default, retries are disabled. Call this method to enable automatic
    /// retries on transient failures with exponential backoff.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// use strata_paas::RetryConfig;
    ///
    /// let retry_config = RetryConfig {
    ///     max_retries: 5,
    ///     base_delay_secs: 10,
    ///     multiplier: 2.0,
    ///     max_delay_secs: 300,
    /// };
    ///
    /// let builder = ProverServiceBuilder::new(config)
    ///     .with_retry_config(retry_config)
    ///     // ... other configuration
    /// ```
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.config.retry = Some(retry_config);
        self
    }

    /// Launch the prover service with all registered handlers
    ///
    /// Creates semaphores for each backend based on worker configuration,
    /// initializes ProverServiceState, and launches the service.
    ///
    /// ## Returns
    ///
    /// Returns `ProverHandle` for interacting with the service, or error if launch fails.
    pub async fn launch(self, executor: &TaskExecutor) -> ProverServiceResult<ProverHandle<P>> {
        // Create semaphores for each backend based on worker count
        let mut semaphores = HashMap::new();

        // SP1 backend semaphore
        let sp1_count = self
            .config
            .workers
            .worker_count
            .get(&ZkVmBackend::SP1)
            .copied()
            .unwrap_or(1);
        semaphores.insert(ZkVmBackend::SP1, Arc::new(Semaphore::new(sp1_count)));

        // Native backend semaphore
        let native_count = self
            .config
            .workers
            .worker_count
            .get(&ZkVmBackend::Native)
            .copied()
            .unwrap_or(1);
        semaphores.insert(ZkVmBackend::Native, Arc::new(Semaphore::new(native_count)));

        // Risc0 backend semaphore
        let risc0_count = self
            .config
            .workers
            .worker_count
            .get(&ZkVmBackend::Risc0)
            .copied()
            .unwrap_or(1);
        semaphores.insert(ZkVmBackend::Risc0, Arc::new(Semaphore::new(risc0_count)));

        // Require task store
        let task_store = self
            .task_store
            .expect("TaskStore must be provided via with_task_store()");

        // Create retry scheduler channel (handle created first, service later)
        let (scheduler_tx, scheduler_rx) = mpsc::unbounded_channel::<SchedulerCommand<P>>();
        let scheduler_handle = SchedulerHandle::new(scheduler_tx);

        // Create ProverServiceState with handlers and semaphores
        let state = ProverServiceState::new(
            self.config,
            task_store,
            self.handlers,
            semaphores,
            executor.clone(),
            scheduler_handle,
        );

        // Create service builder
        let mut service_builder = ServiceBuilder::<ProverService<P>, _>::new().with_state(state);

        // Create command handle (shared between scheduler and ProverHandle)
        let command_handle = Arc::new(service_builder.create_command_handle(100));

        // Now create and spawn retry scheduler with the command handle
        let retry_scheduler =
            RetryScheduler::new(scheduler_rx, Arc::clone(&command_handle), executor.clone());
        executor.spawn_critical_async("paas_retry_scheduler", async move {
            retry_scheduler.run().await;
            Ok(())
        });

        // Launch service
        let monitor = service_builder
            .launch_async("prover", executor)
            .await
            .map_err(ProverServiceError::Internal)?;

        // Return handle
        Ok(ProverHandle::new(command_handle, monitor))
    }
}
