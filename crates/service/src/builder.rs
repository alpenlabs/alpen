//! Service builder/launcher infra.

use tokio::sync::watch;

use crate::*;

pub struct ServiceBuilder<S: Service> {
    state: Option<S::State>,
    inp: Option<S::Input>,
}

impl<S: Service> ServiceBuilder<S> {
    pub fn with_state(mut self, s: S::State) -> Self {
        self.state = Some(s);
        self
    }

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
    ) -> anyhow::Result<StatusHandle<S>> {
        // TODO convert to fallible results?
        let state = self.state.expect("service/builder: missing state");
        let inp = self.inp.expect("service/builder: missing input");

        let init_status = S::get_status(&state);
        let (status_tx, status_rx) = watch::channel(init_status);

        let worker_fut = async_worker::worker_task::<S>(state, inp, status_tx);
        texec.spawn_critical_async(&name, worker_fut);

        Ok(StatusHandle::new(status_rx))
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
    ) -> anyhow::Result<StatusHandle<S>> {
        // TODO convert to fallible results?
        let state = self.state.expect("service/builder: missing state");
        let inp = self.inp.expect("service/builder: missing input");

        let init_status = S::get_status(&state);
        let (status_tx, status_rx) = watch::channel(init_status);

        let worker_cls = move |g| sync_worker::worker_task::<S>(state, inp, status_tx, g);
        texec.spawn_critical(name, worker_cls);

        Ok(StatusHandle::new(status_rx))
    }
}
