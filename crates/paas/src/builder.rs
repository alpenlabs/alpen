//! Builder for direct handler-based prover service

use std::{collections::HashMap, sync::Arc};

use strata_service::ServiceBuilder;
use strata_tasks::TaskExecutor;
use tokio::sync::{mpsc, Semaphore};

use crate::{
    config::ProverServiceConfig,
    error::ProverServiceResult,
    handle::ProverHandle,
    handler::ProofHandler,
    service::ProverService,
    state::ProverServiceState,
    timer::{TimerCommand, TimerHandle, TimerService},
    ProgramType, ZkVmBackend,
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
    task_store: Option<Arc<dyn crate::persistence::TaskStore<P>>>,
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
        S: crate::persistence::TaskStore<P> + 'static,
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
    pub fn with_retry_config(mut self, retry_config: crate::config::RetryConfig) -> Self {
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

        // Create timer service channel
        let (timer_tx, timer_rx) = mpsc::unbounded_channel::<TimerCommand<P>>();
        let timer_handle = TimerHandle::new(timer_tx);

        // Create timer service and spawn it as a critical background task
        let timer_service = TimerService::new(timer_rx, executor.clone());
        executor.spawn_critical_async("paas_timer_service", async move {
            timer_service.run().await;
            Ok(())
        });

        // Create ProverServiceState with handlers and semaphores
        let state = ProverServiceState::new(
            self.config,
            task_store,
            self.handlers,
            semaphores,
            executor.clone(),
            timer_handle,
        );

        // Create service builder
        let mut service_builder = ServiceBuilder::<ProverService<P>, _>::new().with_state(state);

        // Create command handle
        let command_handle = service_builder.create_command_handle(100);

        // Launch service
        let monitor = service_builder
            .launch_async("prover", executor)
            .await
            .map_err(crate::error::ProverServiceError::Internal)?;

        // Return handle
        Ok(ProverHandle::new(command_handle, monitor))
    }
}
