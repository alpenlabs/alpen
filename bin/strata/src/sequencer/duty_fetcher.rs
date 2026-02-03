//! Duty fetcher worker for sequencer.

use std::{sync::Arc, time::Duration};

use strata_ol_sequencer::{Duty, TemplateManager, extract_duties};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info, warn};

/// Worker for fetching duties from the sequencer.
pub(crate) async fn duty_fetcher_worker(
    template_manager: Arc<TemplateManager>,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    duty_tx: mpsc::Sender<Duty>,
    poll_interval: Duration,
) -> anyhow::Result<()> {
    let mut interval = interval(poll_interval);
    'top: loop {
        interval.tick().await;
        let Some(tip_blkid) = status_channel.get_ol_sync_status().map(|s| *s.tip_blkid()) else {
            continue;
        };

        let duties =
            match extract_duties(template_manager.as_ref(), tip_blkid, storage.as_ref()).await {
                Ok(duties) => duties,
                Err(err) => {
                    error!("duty_fetcher_worker: failed to extract duties: {err}");
                    continue;
                }
            };

        info!(count = %duties.len(), "got new duties");

        for duty in duties {
            if duty_tx.send(duty).await.is_err() {
                warn!("duty_fetcher_worker: rx dropped; exiting");
                break 'top;
            }
        }
    }

    Ok(())
}
