use std::future::Future;

use reth_tasks::{shutdown::Shutdown, TaskExecutor};
use strata_service::{AsyncExecutor, AsyncGuard};

pub(crate) struct ServiceExecutor {
    inner: TaskExecutor,
}

impl ServiceExecutor {
    pub(crate) fn from_reth(inner: TaskExecutor) -> Self {
        Self { inner }
    }
}

impl AsyncExecutor for ServiceExecutor {
    type ShutdownGuard = ServiceShutdownGuard;

    fn spawn_async<F>(
        &self,
        name: &'static str,
        worker: impl FnOnce(Self::ShutdownGuard) -> F + Send + 'static,
    ) where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        self.inner
            .spawn_critical_with_shutdown_signal(name, |shutdown| async move {
                worker(ServiceShutdownGuard(shutdown))
                    .await
                    .expect("critical service should not error")
            });
    }
}

pub(crate) struct ServiceShutdownGuard(Shutdown);

impl AsyncGuard for ServiceShutdownGuard {
    fn wait_for_shutdown(&self) -> impl Future<Output = ()> {
        self.0.clone()
    }
}
