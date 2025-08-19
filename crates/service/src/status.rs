//! Status handle.

use std::{any::Any, fmt::Debug, sync::Arc};

use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::watch;

use crate::Service;

/// Generic boxed status value which can be downcast to a concrete type.
pub type AnyStatus = Box<dyn Any + Sync + Send + 'static>;

/// Service status monitor handle.
#[derive(Clone, Debug)]
pub struct ServiceMonitor<S: Service> {
    status_rx: watch::Receiver<S::Status>,
}

impl<S: Service> ServiceMonitor<S> {
    pub(crate) fn new(status_rx: watch::Receiver<S::Status>) -> Self {
        Self { status_rx }
    }

    /// Returns a clone of the current status.
    pub fn get_current(&self) -> S::Status {
        self.status_rx.borrow().clone()
    }

    /// Converts this service monitor into a generic one.
    pub fn into_generic(self) -> GenericStatusMonitor {
        GenericStatusMonitor::new(self)
    }
}

/// Service monitor type.
///
/// This is intended to be object-safe so that we can have a collection of
/// monitors for heterogeneous service types.
pub trait StatusMonitor {
    /// Fetches the latest status as a boxed `dyn Any`.
    fn fetch_status_any(&mut self) -> anyhow::Result<AnyStatus>;

    /// Fetches the latest status as a JSON value.
    fn fetch_status_json(&mut self) -> anyhow::Result<serde_json::Value>;
}

impl<S: Service> StatusMonitor for ServiceMonitor<S> {
    fn fetch_status_any(&mut self) -> anyhow::Result<AnyStatus> {
        let v = self.status_rx.borrow();
        Ok(Box::new(v.clone()))
    }

    fn fetch_status_json(&mut self) -> anyhow::Result<serde_json::Value> {
        let v = self.status_rx.borrow();
        Ok(serde_json::to_value(&*v)?)
    }
}

/// Generic status monitor for an arbitrary service.
///
/// This exists to make it easier to work with a collection of status monitors
/// for unrelated services and reduce the burden of dealing with all the types.
#[allow(missing_debug_implementations)]
pub struct GenericStatusMonitor {
    inner: Box<dyn StatusMonitor>,
}

impl GenericStatusMonitor {
    /// Creates a new instance from a specific service monitor.
    pub fn new<S: Service>(inner: ServiceMonitor<S>) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    /// Fetches the latest status as a boxed `dyn Any`.
    pub fn fetch_status_any(&mut self) -> anyhow::Result<AnyStatus> {
        self.inner.fetch_status_any()
    }

    /// Fetches the latest status as a JSON value.
    pub fn fetch_status_json(&mut self) -> anyhow::Result<serde_json::Value> {
        self.inner.fetch_status_json()
    }

    /// Tries to extract and deserialize a field from the JSON-encoded value
    /// of the status, if it exists and is compatible with the type.
    // FIXME this is a kinda expensive thing to do, we should be able to do this
    // with `Serializer` hacks and `Box<dyn Any>` downcasting
    pub fn query_status_field<T: DeserializeOwned>(
        &mut self,
        k: &str,
    ) -> anyhow::Result<Option<T>> {
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
