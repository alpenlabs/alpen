//! Service builder/launcher infra.

use std::fmt::Debug;

use tokio::sync::{mpsc, watch};

use crate::{
    async_worker, sync_worker, AsyncService, AsyncServiceInput, CommandHandle, Service,
    ServiceInput, ServiceMonitor, ServiceMsg, SyncService, SyncServiceInput, TokioMpscInput,
};

/// Builder to help with constructing service workers.
#[derive(Debug)]
pub struct ServiceBuilder<S: Service, I> {
    state: Option<S::State>,
    inp: Option<I>,
}

impl<S: Service, I> ServiceBuilder<S, I> {
    /// Constructs an uninitialized service builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the service's state.
    pub fn with_state(mut self, s: S::State) -> Self {
        self.state = Some(s);
        self
    }

    /// Returns if we're ready to start the service.  Further changes MAY be
    /// able to be made after this starts returning true.
    ///
    /// If this returns `true`, then launch fns MUST NOT fail.
    pub fn is_ready(&self) -> bool {
        self.state.is_some() && self.inp.is_some()
    }
}

impl<S: Service, I: ServiceInput<Msg = S::Msg>> ServiceBuilder<S, I> {
    /// Sets the input that will be used with the service.
    pub fn with_input(mut self, inp: I) -> Self {
        self.inp = Some(inp);
        self
    }
}

impl<S: AsyncService, I> ServiceBuilder<S, I>
where
    I: AsyncServiceInput<Msg = S::Msg>,
{
    /// Launches the async service task in an executor.
    pub async fn launch_async(
        self,
        name: &'static str,
        texec: &strata_tasks::TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<S>> {
        // TODO convert to fallible results?
        let state = self.state.expect("service/builder: missing state");
        let inp = self.inp.expect("service/builder: missing input");

        let init_status = S::get_status(&state);
        let (status_tx, status_rx) = watch::channel(init_status);

        let worker_fut_cls = move |g| async_worker::worker_task::<S, I>(state, inp, status_tx, g);
        texec.spawn_critical_async_with_shutdown(name, worker_fut_cls);

        Ok(ServiceMonitor::new(status_rx))
    }
}

impl<S: SyncService, I> ServiceBuilder<S, I>
where
    I: SyncServiceInput<Msg = S::Msg>,
{
    /// Launches the service thread in an executor.
    pub fn launch_sync(
        self,
        name: &'static str,
        texec: &strata_tasks::TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<S>> {
        // TODO convert to fallible results?
        let state = self.state.expect("service/builder: missing state");
        let inp = self.inp.expect("service/builder: missing input");

        let init_status = S::get_status(&state);
        let (status_tx, status_rx) = watch::channel(init_status);

        let worker_cls = move |g| sync_worker::worker_task::<S, I>(state, inp, status_tx, g);
        texec.spawn_critical(name, worker_cls);

        Ok(ServiceMonitor::new(status_rx))
    }
}

/// Specialized impl to construct a command worker and return a command handle.
impl<T: ServiceMsg, S: Service<Msg = T>> ServiceBuilder<S, TokioMpscInput<T>> {
    /// Returns a command input handle that can be used to send inputs to the
    /// worker when it's launched.
    ///
    /// This being a standalone function allows the caller to get the handle to
    /// the task before the task is actually started.  This capability must be
    /// handled with care to avoid creating complex interdependencies between
    /// services.
    ///
    /// # Panics
    ///
    /// If an input has already been set.
    pub fn create_command_handle(&mut self, capacity: usize) -> CommandHandle<T> {
        if self.inp.is_some() {
            panic!("service/builder: input already created");
        }

        let (tx, rx) = mpsc::channel(capacity);

        let input = TokioMpscInput::new(rx);
        self.inp = Some(input);

        CommandHandle::new(tx)
    }
}

impl<S: Service, I> Default for ServiceBuilder<S, I> {
    fn default() -> Self {
        Self {
            state: None,
            inp: None,
        }
    }
}
