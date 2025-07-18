use std::sync::Arc;

use strata_eectl::handle::ExecCtlHandle;
use strata_primitives::params::Params;
use strata_status::StatusChannel;
use strata_tasks::TaskExecutor;
use tokio::{
    runtime::Handle,
    sync::{Mutex, mpsc},
};

use crate::{
    ChainWorkerHandle, ChainWorkerMessage, WorkerContext, WorkerError, WorkerResult, WorkerShared,
    worker,
};

/// Builder for creating and launching a chain worker task.
///
/// This encapsulates all the initialization logic and dependencies needed
/// to spawn a chain worker, preventing implementation details from leaking
/// into the caller. The builder launches the task and returns a handle to it.
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

    /// Launch the chain worker task and return a handle to it.
    ///
    /// This method validates all required dependencies, creates the necessary
    /// channels, spawns the worker task using the provided executor, and returns
    /// a handle for interacting with the worker.
    pub fn launch(self, executor: &TaskExecutor) -> WorkerResult<ChainWorkerHandle>
    where
        W: WorkerContext + Send + 'static,
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

        // Create the message channel for communication with the worker
        let (msg_tx, msg_rx) = mpsc::channel::<ChainWorkerMessage>(64);

        // Create shared state for the worker
        let shared = Arc::new(Mutex::new(WorkerShared::default()));

        // Create the handle that will be returned
        let handle = ChainWorkerHandle::new(shared.clone(), msg_tx);

        // Spawn the worker task
        executor.spawn_critical("chain_worker_task", move |shutdown| {
            worker::worker_task(
                shutdown,
                runtime_handle,
                context,
                status_channel,
                params,
                exec_ctl_handle,
                msg_rx,
                shared,
            )
        });

        Ok(handle)
    }
}

impl<W> Default for ChainWorkerBuilder<W> {
    fn default() -> Self {
        Self::new()
    }
}
