//! OL checkpoint builder worker.
//!
//! This module provides a worker that monitors for new epoch summaries
//! and triggers checkpoint building when they become available.

use strata_status::StatusChannel;
use strata_tasks::ShutdownGuard;
use tokio::runtime::Handle;
use tracing::{debug, info, warn};

use crate::handle::OLCheckpointHandle;

/// Runs the OL checkpoint worker loop.
///
/// This worker monitors chain sync status updates and triggers checkpoint
/// building when new epoch summaries are available.
///
/// Designed to be spawned via `TaskExecutor::spawn_critical`:
/// ```ignore
/// executor.spawn_critical("ol_checkpoint_worker", |shutdown| {
///     ol_checkpoint_worker(shutdown, status_ch.clone(), handle.clone(), rt.clone())
/// });
/// ```
pub fn ol_checkpoint_worker(
    shutdown: ShutdownGuard,
    status_ch: StatusChannel,
    handle: OLCheckpointHandle,
    rt: Handle,
) -> anyhow::Result<()> {
    info!("starting OL checkpoint worker");

    // Subscribe to chain sync updates
    let mut sync_rx = status_ch.subscribe_chain_sync();

    loop {
        if shutdown.should_shutdown() {
            warn!("OL checkpoint worker received shutdown signal");
            break;
        }

        // Wait for a chain sync update (blocking on tokio watch)
        let changed = rt.block_on(sync_rx.changed());
        if changed.is_err() {
            debug!("chain sync channel closed, exiting");
            break;
        }

        // Check for shutdown again before processing
        if shutdown.should_shutdown() {
            warn!("OL checkpoint worker received shutdown signal");
            break;
        }

        // Trigger checkpoint processing
        debug!("received chain sync update, triggering checkpoint tick");
        if let Err(e) = handle.tick_blocking() {
            warn!(err = %e, "checkpoint tick failed");
        }
    }

    info!("OL checkpoint worker exiting");
    Ok(())
}
