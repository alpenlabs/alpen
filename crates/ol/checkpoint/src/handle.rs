//! Handle for interacting with the OL checkpoint worker.

use strata_service::ServiceMonitor;

use crate::service::OLCheckpointStatus;

/// Handle for interacting with the OL checkpoint worker.
#[derive(Debug, Clone)]
pub struct OLCheckpointWorkerHandle {
    monitor: ServiceMonitor<OLCheckpointStatus>,
}

impl OLCheckpointWorkerHandle {
    pub fn new(monitor: ServiceMonitor<OLCheckpointStatus>) -> Self {
        Self { monitor }
    }

    pub fn status(&self) -> OLCheckpointStatus {
        self.monitor.get_current()
    }
}
