//! Status handle.

use tokio::sync::watch;

use super::Service;

/// Service status monitor handle.
pub struct StatusHandle<S: Service> {
    status_rx: watch::Receiver<S::Status>,
}
