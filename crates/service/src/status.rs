//! Status handle.

use std::{any::Any, fmt::Debug};

use serde::de::DeserializeOwned;
use tokio::sync::{mpsc, watch};

use crate::{ServiceStatus, TokioMpscInput};

/// Generic boxed status value which can be downcast to a concrete type.
pub type AnyStatus = Box<dyn Any + Sync + Send + 'static>;

/// Service status monitor handle.
#[derive(Clone, Debug)]
pub struct ServiceMonitor<S: ServiceStatus> {
    pub(crate) status_rx: watch::Receiver<S>,
}

impl<S: ServiceStatus> ServiceMonitor<S> {
    pub(crate) fn new(status_rx: watch::Receiver<S>) -> Self {
        Self { status_rx }
    }

    /// Returns a clone of the current status.
    pub fn get_current(&self) -> S {
        self.status_rx.borrow().clone()
    }

    /// Converts this service monitor into a generic one.
    pub fn into_generic(self) -> GenericStatusMonitor {
        GenericStatusMonitor::new(self)
    }

    /// Creates a listener input for this service monitor.
    ///
    /// This allows another service to listen to status updates from this service.
    /// Returns an MPSC receiver that will receive status updates, and spawns a task
    /// to forward watch channel updates to it.
    ///
    /// The listener task will automatically exit when the monitored service exits.
    pub fn create_listener_input(
        &self,
        executor: &strata_tasks::TaskExecutor,
    ) -> TokioMpscInput<S> {
        // Create an MPSC channel for forwarding status updates
        let (tx, rx) = mpsc::channel(100);

        // Clone the watch receiver
        let mut watch_rx = self.status_rx.clone();

        // Spawn a task to forward watch updates to MPSC
        // This is a non-critical background task that will exit when either:
        // - The monitored service exits (watch channel closes)
        // - The listener service exits (MPSC receiver dropped)
        executor.handle().spawn(async move {
            while watch_rx.changed().await.is_ok() {
                // Get the new status and send it
                let status = watch_rx.borrow_and_update().clone();
                if tx.send(status).await.is_err() {
                    // Receiver dropped, exit
                    break;
                }
            }
            // Watch channel closed - monitored service exited
        });

        TokioMpscInput::new(rx)
    }
}

/// Service monitor type.
///
/// This is intended to be object-safe so that we can have a collection of
/// monitors for heterogeneous service types.
pub trait StatusMonitor {
    /// Fetches the latest status as a boxed `dyn Any`.
    fn fetch_status_any(&self) -> anyhow::Result<AnyStatus>;

    /// Fetches the latest status as a JSON value.
    fn fetch_status_json(&self) -> anyhow::Result<serde_json::Value>;
}

impl<S: ServiceStatus> StatusMonitor for ServiceMonitor<S> {
    fn fetch_status_any(&self) -> anyhow::Result<AnyStatus> {
        let v = self.status_rx.borrow();
        Ok(Box::new(v.clone()))
    }

    fn fetch_status_json(&self) -> anyhow::Result<serde_json::Value> {
        let v = self.status_rx.borrow();
        Ok(serde_json::to_value(&*v)?)
    }
}

/// Generic status monitor for an arbitrary service.
///
/// This exists to make it easier to work with a collection of status monitors
/// for unrelated services and reduce the burden of dealing with all the types.
#[expect(
    missing_debug_implementations,
    reason = "Trait object cannot implement Debug"
)]
pub struct GenericStatusMonitor {
    inner: Box<dyn StatusMonitor>,
}

impl GenericStatusMonitor {
    /// Creates a new instance from a specific service monitor.
    pub fn new<S: ServiceStatus>(inner: ServiceMonitor<S>) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    /// Fetches the latest status as a boxed `dyn Any`.
    pub fn fetch_status_any(&self) -> anyhow::Result<AnyStatus> {
        self.inner.fetch_status_any()
    }

    /// Fetches the latest status as a JSON value.
    pub fn fetch_status_json(&self) -> anyhow::Result<serde_json::Value> {
        self.inner.fetch_status_json()
    }

    /// Tries to extract and deserialize a field from the JSON-encoded value
    /// of the status, if it exists and is compatible with the type.
    // FIXME this is a kinda expensive thing to do, we should be able to do this
    // with `Serializer` hacks and `Box<dyn Any>` downcasting
    pub fn query_status_field<T: DeserializeOwned>(&self, k: &str) -> anyhow::Result<Option<T>> {
        let j = self.inner.fetch_status_json()?;

        let serde_json::Value::Object(mut obj) = j else {
            return Ok(None);
        };

        // Removing this because this is an ephemeral JSON value we just created
        // and we need to consume the value.
        let Some(v) = obj.remove(k) else {
            return Ok(None);
        };

        Ok(Some(serde_json::from_value::<T>(v)?))
    }
}
