//! Builder pattern for launching the chain worker service.

use std::{fmt::Debug, sync::Arc};

use strata_ol_state_types::OLState;
use strata_params::Params;
use strata_service::ServiceBuilder;
use strata_status::StatusChannel;
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;

use crate::{
    errors::{WorkerError, WorkerResult},
    handle::ChainWorkerHandle,
    service::ChainWorkerService,
    state::ChainWorkerServiceState,
    traits::ChainWorkerContext,
};

/// Builder for constructing and launching a chain worker service.
///
/// This encapsulates all the initialization logic and dependencies needed to
/// launch a chain worker using the service framework, preventing impl details
/// from leaking into the caller. The builder launches the service and returns
/// a handle to it.
///
/// Generic over the state type for `StatusChannel`, defaulting to `OLState`.
///
/// # Example
///
/// ```ignore
/// let handle = ChainWorkerBuilder::new()
///     .with_context(context)
///     .with_params(params)
///     .with_status_channel(status_channel)
///     .with_runtime(runtime_handle)
///     .launch(&executor)?;
/// ```
#[derive(Debug)]
pub struct ChainWorkerBuilder<
    W: ChainWorkerContext + Send + Sync + 'static,
    State: Clone + Debug + Send + Sync + 'static = OLState,
> {
    context: Option<W>,
    params: Option<Arc<Params>>,
    status_channel: Option<StatusChannel<State>>,
    runtime_handle: Option<Handle>,
}

impl<W: ChainWorkerContext + Send + Sync + 'static, State: Clone + Debug + Send + Sync + 'static>
    ChainWorkerBuilder<W, State>
{
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            context: None,
            params: None,
            status_channel: None,
            runtime_handle: None,
        }
    }

    /// Set the worker context (implements [`ChainWorkerContext`] trait).
    pub fn with_context(mut self, context: W) -> Self {
        self.context = Some(context);
        self
    }

    /// Set the rollup parameters.
    pub fn with_params(mut self, params: Arc<Params>) -> Self {
        self.params = Some(params);
        self
    }

    /// Set the status channel for genesis waiting.
    pub fn with_status_channel(mut self, channel: StatusChannel<State>) -> Self {
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
    /// uses [`ServiceBuilder`] to set up the service infrastructure, and returns
    /// a handle for interacting with the worker.
    pub fn launch(self, executor: &TaskExecutor) -> WorkerResult<ChainWorkerHandle>
    where
        W: ChainWorkerContext + Send + Sync + 'static,
    {
        let context = self
            .context
            .ok_or(WorkerError::MissingDependency("context"))?;
        let params = self
            .params
            .ok_or(WorkerError::MissingDependency("params"))?;
        let status_channel = self
            .status_channel
            .ok_or(WorkerError::MissingDependency("status_channel"))?;
        let runtime_handle = self
            .runtime_handle
            .ok_or(WorkerError::MissingDependency("runtime_handle"))?;

        // Create the service state.
        let service_state =
            ChainWorkerServiceState::new(context, params, status_channel, runtime_handle);

        // Create the service builder and get command handle.
        let mut service_builder =
            ServiceBuilder::<ChainWorkerService<W, State>, _>::new().with_state(service_state);

        // Create the command handle before launching.
        let command_handle = service_builder.create_command_handle(64);

        // Launch the service using the sync worker.
        let _service_monitor = service_builder
            .launch_sync("chain_worker_new", executor)
            .map_err(|e| WorkerError::Unexpected(format!("failed to launch service: {}", e)))?;

        // Create and return the handle.
        let handle = ChainWorkerHandle::new(command_handle);

        Ok(handle)
    }
}

impl<W: ChainWorkerContext + Send + Sync + 'static> Default for ChainWorkerBuilder<W> {
    fn default() -> Self {
        Self::new()
    }
}
