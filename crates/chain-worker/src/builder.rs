use std::sync::Arc;

use strata_eectl::handle::ExecCtlHandle;
use strata_primitives::params::Params;
use strata_service::ServiceBuilder;
use strata_status::StatusChannel;
use strata_tasks::TaskExecutor;
use tokio::{runtime::Handle, sync::Mutex};

use crate::{
    service::{ChainWorkerService, ChainWorkerServiceState},
    handle::{ChainWorkerHandle, WorkerShared},
    traits::WorkerContext,
    errors::{WorkerError, WorkerResult},
};

/// Builder for creating and launching a chain worker service.
///
/// This encapsulates all the initialization logic and dependencies needed
/// to spawn a chain worker using the service framework, preventing implementation
/// details from leaking into the caller. The builder launches the service and 
/// returns a handle to it.
#[derive(Debug)]
pub struct ChainWorkerBuilder<W> {
    context: Option<W>,
    params: Option<Arc<Params>>,
    exec_ctl_handle: Option<ExecCtlHandle>,
    status_channel: Option<StatusChannel>,
    runtime_handle: Option<Handle>,
}

impl<W> ChainWorkerBuilder<W> {
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            context: None,
            params: None,
            exec_ctl_handle: None,
            status_channel: None,
            runtime_handle: None,
        }
    }

    /// Set the worker context (implements WorkerContext trait).
    pub fn with_context(mut self, context: W) -> Self {
        self.context = Some(context);
        self
    }

    /// Set the rollup parameters.
    pub fn with_params(mut self, params: Arc<Params>) -> Self {
        self.params = Some(params);
        self
    }

    /// Set the execution control handle.
    pub fn with_exec_handle(mut self, handle: ExecCtlHandle) -> Self {
        self.exec_ctl_handle = Some(handle);
        self
    }

    /// Set the status channel for genesis waiting.
    pub fn with_status_channel(mut self, channel: StatusChannel) -> Self {
        self.status_channel = Some(channel);
        self
    }

    /// Set the runtime handle for blocking operations.
    pub fn with_runtime(mut self, handle: Handle) -> Self {
        self.runtime_handle = Some(handle);
        self
    }

    /// Launch the chain worker service and return a handle to it.
    ///
    /// This method validates all required dependencies, creates the service state,
    /// uses ServiceBuilder to set up the service infrastructure, and returns
    /// a handle for interacting with the worker.
    pub fn launch(self, executor: &TaskExecutor) -> WorkerResult<ChainWorkerHandle>
    where
        W: WorkerContext + Send + Sync + 'static,
    {
        let context = self
            .context
            .ok_or(WorkerError::MissingDependency("context"))?;
        let params = self
            .params
            .ok_or(WorkerError::MissingDependency("params"))?;
        let exec_ctl_handle = self
            .exec_ctl_handle
            .ok_or(WorkerError::MissingDependency("exec_ctl_handle"))?;
        let status_channel = self
            .status_channel
            .ok_or(WorkerError::MissingDependency("status_channel"))?;
        let runtime_handle = self
            .runtime_handle
            .ok_or(WorkerError::MissingDependency("runtime_handle"))?;

        // Create shared state for the worker
        let shared = Arc::new(Mutex::new(WorkerShared::default()));

        // Create the service state
        let service_state = ChainWorkerServiceState::new(
            shared.clone(),
            context,
            params,
            exec_ctl_handle,
            status_channel,
            runtime_handle,
        );

        // Create the service builder and get command handle
        let mut service_builder = ServiceBuilder::<ChainWorkerService<W>, _>::new()
            .with_state(service_state);

        // Create the command handle before launching
        let command_handle = service_builder.create_command_handle(64);

        // Launch the service using the sync worker
        let _service_monitor = service_builder
            .launch_sync("chain_worker", executor)
            .map_err(|e| WorkerError::Unexpected(format!("failed to launch service: {}", e)))?;

        // Create and return the handle
        let handle = ChainWorkerHandle::new(shared, command_handle);

        Ok(handle)
    }
}

impl<W> Default for ChainWorkerBuilder<W> {
    fn default() -> Self {
        Self::new()
    }
}
