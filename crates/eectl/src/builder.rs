use std::{fmt::Debug, sync::Arc};

use strata_status::StatusChannel;
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;

use crate::{
    engine::ExecEngineCtl,
    errors::{EngineError, EngineResult},
    handle::{make_handle_pair, ExecCtlHandle},
    worker::{worker_task, ExecWorkerContext},
};

/// Builder for creating and launching an exec worker task.
///
/// This encapsulates all the initialization logic and dependencies needed
/// to spawn an exec worker, preventing implementation details from leaking
/// into the caller. The builder launches the task and returns a result.
///
/// Generic over the state type for `StatusChannel`, defaulting to `Chainstate`.
#[derive(Debug)]
pub struct ExecWorkerBuilder<E, W> {
    context: Option<W>,
    engine: Option<Arc<E>>,
    status_channel: Option<StatusChannel>,
    runtime_handle: Option<Handle>,
}

impl<E, W> ExecWorkerBuilder<E, W> {
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            context: None,
            engine: None,
            status_channel: None,
            runtime_handle: None,
        }
    }

    /// Set the node storage.
    pub fn with_context(mut self, context: W) -> Self {
        self.context = Some(context);
        self
    }

    /// Set the execution engine.
    pub fn with_engine(mut self, engine: Arc<E>) -> Self {
        self.engine = Some(engine);
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

    /// Launch the exec worker task.
    ///
    /// This method validates all required dependencies, initializes the worker state,
    /// and spawns the worker task using the provided executor.
    pub fn launch(self, executor: &TaskExecutor) -> EngineResult<ExecCtlHandle>
    where
        E: ExecEngineCtl + Sync + Send + 'static,
        W: ExecWorkerContext + Sync + Send + 'static,
    {
        let engine = self
            .engine
            .ok_or(EngineError::MissingDependency("engine"))?;
        let context = self
            .context
            .ok_or(EngineError::MissingDependency("context"))?;
        let status_channel = self
            .status_channel
            .ok_or(EngineError::MissingDependency("status_channel"))?;
        let runtime_handle = self
            .runtime_handle
            .ok_or(EngineError::MissingDependency("runtime_handle"))?;

        // Create the message channel for communication with the worker
        let (exec_tx, exec_rx) = make_handle_pair();

        // Spawn the worker task
        executor.spawn_critical("exec_worker_task", move |shutdown| {
            worker_task(
                shutdown,
                runtime_handle,
                &context,
                status_channel,
                engine,
                exec_rx,
            )
        });

        Ok(exec_tx)
    }
}

impl<E, W> Default for ExecWorkerBuilder<E, W> {
    fn default() -> Self {
        Self::new()
    }
}
