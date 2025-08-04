//! Service builder/launcher infra.

use tokio::sync::watch;

use crate::*;

/// Builder to help with constructing service workers.
#[derive(Debug)]
pub struct ServiceBuilder<S: Service> {
    state: Option<S::State>,
    inp: Option<S::Input>,
}

impl<S: Service> ServiceBuilder<S> {
    /// Sets the service's state.
    pub fn with_state(mut self, s: S::State) -> Self {
        self.state = Some(s);
        self
    }

    /// Sets the input that will be used with the service.
    pub fn with_input(mut self, inp: S::Input) -> Self {
        self.inp = Some(inp);
        self
    }
}

impl<S: Service + AsyncService> ServiceBuilder<S>
where
    S::Input: AsyncServiceInput + Sync + Send,
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

        let worker_fut_cls = move |g| async_worker::worker_task::<S>(state, inp, status_tx, g);
        texec.spawn_critical_async_with_shutdown(&name, worker_fut_cls);

        Ok(ServiceMonitor::new(status_rx))
    }
}

impl<S: Service + SyncService> ServiceBuilder<S>
where
    S::Input: SyncServiceInput,
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

        let worker_cls = move |g| sync_worker::worker_task::<S>(state, inp, status_tx, g);
        texec.spawn_critical(name, worker_cls);

        Ok(ServiceMonitor::new(status_rx))
    }
}
