//! Handle for interacting with the OL checkpoint service.

use strata_service::ServiceMonitor;

use crate::service::OLCheckpointStatus;

/// Handle for interacting with the OL checkpoint service.
#[derive(Debug, Clone)]
pub struct OLCheckpointHandle {
    monitor: ServiceMonitor<OLCheckpointStatus>,
}

impl OLCheckpointHandle {
    pub fn new(monitor: ServiceMonitor<OLCheckpointStatus>) -> Self {
        Self { monitor }
    }

    pub fn status(&self) -> OLCheckpointStatus {
        self.monitor.get_current()
    }
}
