//! Duty fetcher worker for sequencer.

use std::{sync::Arc, time::Duration};

use strata_ol_sequencer::{Duty, TemplateManager, extract_duties};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::{sync::mpsc, time::interval};
use tracing::{debug, error, warn};

/// Worker for fetching duties from the sequencer.
#[tracing::instrument(skip_all, fields(component = "sequencer_duty_fetcher"))]
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
        let tip_blkid = match status_channel.get_ol_sync_status().map(|s| *s.tip_blkid()) {
            Some(tip) => tip,
            None => match storage.ol_block().get_canonical_block_at_async(0).await {
                Ok(Some(commitment)) => *commitment.blkid(),
                Ok(None) => {
                    warn!("genesis block not found yet");
                    continue;
                }
                Err(err) => {
                    error!(%err, "failed to load genesis block");
                    continue;
                }
            },
        };

        let duties =
            match extract_duties(template_manager.as_ref(), tip_blkid, storage.as_ref()).await {
                Ok(duties) => duties,
                Err(err) => {
                    error!(%err, "failed to extract duties");
                    continue;
                }
            };

        if duties.is_empty() {
            debug!(count = %duties.len(), "got no new duties, skipping");
            continue;
        }

        // Log non-empty duties
        let duties_display: Vec<String> = duties.iter().map(ToString::to_string).collect();
        debug!(duties = ?duties_display, "got some sequencer duties");

        for duty in duties {
            if duty_tx.send(duty).await.is_err() {
                warn!("duty receiver dropped; exiting");
                break 'top;
            }
        }
    }

    Ok(())
}
