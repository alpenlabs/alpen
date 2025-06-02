use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::sync::{futures::Notified, Notify};

use crate::pending_tasks::PendingTasks;

/// Allows to send a signal to trigger shutdown
#[derive(Debug, Clone)]
pub struct ShutdownSignal(Arc<AtomicBool>, Arc<Notify>);

impl ShutdownSignal {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)), Arc::new(Notify::new()))
    }

    /// Send shutdown signal
    pub fn send(&self) {
        self.0.fetch_or(true, Ordering::Relaxed);
        self.1.notify_waiters();
    }

    pub(crate) fn subscribe(&self) -> Shutdown {
        Shutdown(self.clone())
    }

    fn should_shutdown(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn notified(&self) -> Notified<'_> {
        self.1.notified()
    }
}

/// Receiver for shutdown signal
#[derive(Debug)]
pub(crate) struct Shutdown(ShutdownSignal);

impl Shutdown {
    fn should_shutdown(&self) -> bool {
        self.0.should_shutdown()
    }

    pub(crate) async fn wait_for_shutdown(&self) {
        while !self.should_shutdown() {
            self.0.notified().await
        }
    }
}

/// Receiver for shutdown signal.
/// Also manages an atomic counter to keep track of live tasks.
#[derive(Debug)]
pub struct ShutdownGuard(Shutdown, Arc<PendingTasks>);

impl ShutdownGuard {
    pub(crate) fn new(shutdown: Shutdown, counter: Arc<PendingTasks>) -> Self {
        counter.increment();
        Self(shutdown, counter)
    }

    /// Check if shutdown signal has been sent
    pub fn should_shutdown(&self) -> bool {
        self.0.should_shutdown()
    }

    /// Waits until shutdown signal is sent
    pub async fn wait_for_shutdown(&self) {
        self.0.wait_for_shutdown().await
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.1.decrement();
    }
}
