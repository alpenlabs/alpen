//! Status handle.

use tokio::sync::watch;

use crate::Service;

/// Service status monitor handle.
#[derive(Debug)]
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
}

/// Service monitor type.
///
/// This is intended to be object-safe so that we can have a collection of
/// monitors for heterogeneous service types.
pub trait StatusMonitor {
    /// Fetches the latest status as a JSON value.
    fn fetch_status(&mut self) -> anyhow::Result<serde_json::Value>;
}

impl<S: Service> StatusMonitor for ServiceMonitor<S> {
    fn fetch_status(&mut self) -> anyhow::Result<serde_json::Value> {
        let v = self.status_rx.borrow();
        Ok(serde_json::to_value(&*v)?)
    }
}
