use std::{sync::Arc, time::Duration};

use strata_config::btcio::WriterConfig;
use strata_db_types::{
    types::{BundledPayloadEntry, IntentEntry, IntentStatus},
    DbResult,
};
use strata_storage::ops::writer::EnvelopeDataOps;
use strata_tasks::ShutdownGuard;
use tokio::{select, sync::mpsc::Receiver, time::interval};
use tracing::*;

/// Periodically bundles unbundled intents into payload entries.
pub(crate) async fn bundler_task(
    mut unbundled: Vec<IntentEntry>,
    ops: Arc<EnvelopeDataOps>,
    config: Arc<WriterConfig>,
    mut intent_rx: Receiver<IntentEntry>,
    shutdown: ShutdownGuard,
) -> anyhow::Result<()> {
    let interval = interval(Duration::from_millis(config.bundle_interval_ms));
    tokio::pin!(interval);
    loop {
        select! {
            maybe_intent = intent_rx.recv() => {
                if shutdown.should_shutdown() {
                    info!("Bundler received shutdown. Stopping.");
                    break;
                }
                if let Some(intent) = maybe_intent {
                    unbundled.push(intent);
                } else {
                    warn!("Intent receiver closed, stopping bundler task");
                    break;
                }
            }

            _ = interval.tick() => {
                if shutdown.should_shutdown() {
                    info!("Bundler received shutdown. Stopping.");
                    break;
                }
                // Process unbundled entries, returning entries which are unprocessed for some reason.
                unbundled = process_unbundled_entries(ops.as_ref(), unbundled).await?;
            }
        }
    }
    Ok(())
}

/// Processes and bundles a list of unbundled intents into payload entries. Returns a vector of
/// entries which are unbundled for some reason.
/// The reason could be the entries is too small in size to be included in an envelope and thus
/// makes sense to include once a bunch of entries are collected.
/// NOTE: The current logic is simply 1-1 mapping between intents and payloads, in future it can
/// be sophisticated.
async fn process_unbundled_entries(
    ops: &EnvelopeDataOps,
    unbundled: Vec<IntentEntry>,
) -> DbResult<Vec<IntentEntry>> {
    for mut entry in unbundled {
        // Check it is actually unbundled, omit if bundled
        if entry.status != IntentStatus::Unbundled {
            continue;
        }
        // NOTE: In future, the logic to create payload will be different. We need to group
        // intents and create payload entries accordingly
        let payload_entry = BundledPayloadEntry::new_unsigned(entry.payload().clone());

        // TODO: the following block till "Atomic Ends" should be atomic.
        let idx = ops.get_next_payload_idx_async().await?;
        ops.put_payload_entry_async(idx, payload_entry).await?;

        // Set the entry to be bundled so that it won't be processed next time.
        entry.status = IntentStatus::Bundled(idx);
        ops.put_intent_entry_async(*entry.intent.commitment(), entry)
            .await?;
        // Atomic Ends.
    }
    // Return empty Vec because each entry is being bundled right now. This might be different in
    // future.
    Ok(vec![])
}

/// Retrieves unbundled intents since the beginning in ascending order along with the latest
/// entry idx. This traverses backwards from latest index and breaks once it founds a bundled entry.
pub(crate) fn get_initial_unbundled_entries(
    ops: &EnvelopeDataOps,
) -> anyhow::Result<Vec<IntentEntry>> {
    let mut curr_idx = ops.get_next_intent_idx_blocking()?;
    let mut unbundled = Vec::new();

    while curr_idx > 0 {
        curr_idx -= 1;
        if let Some(intent) = ops.get_intent_by_idx_blocking(curr_idx)? {
            match intent.status {
                IntentStatus::Unbundled => unbundled.push(intent),
                IntentStatus::Bundled(_) => {
                    // Bundled intent found, no more to scan
                    break;
                }
            }
        } else {
            warn!(%curr_idx, "Could not find expected intent in db");
            break;
        }
    }

    // Reverse the items so that they are in ascending order of index
    unbundled.reverse();

    Ok(unbundled)
}
