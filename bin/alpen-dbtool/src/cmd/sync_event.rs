use std::sync::Arc;

use clap::Args;
use strata_db::traits::{Database, L1Database, SyncEventDatabase};
use strata_rocksdb::CommonDb;
use strata_state::sync_event::SyncEvent;

use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to show details about a specific sync event.
#[derive(Args, Debug)]
pub(crate) struct GetSyncEventArgs {
    /// Sync event index; defaults to the last written index
    #[arg(value_name = "SYNC_EVENT_INDEX")]
    pub(crate) event_index: Option<u64>,
}

/// Arguments to show a summary of all sync events.
#[derive(Args, Debug)]
pub(crate) struct GetSyncEventsSummaryArgs {}

/// Get SyncEvent details by index.
pub(crate) fn get_sync_event(
    db: Arc<CommonDb>,
    args: GetSyncEventArgs,
) -> Result<(), DisplayedError> {
    let sync_db = db.sync_event_db();
    let event_index = args.event_index.unwrap_or(
        db.sync_event_db()
            .get_last_idx()
            .internal_error("Failed to get last sync event index")?
            .expect("valid event index"),
    );

    let sync_event = sync_db
        .get_sync_event(event_index)
        .internal_error(format!("Failed to get sync event at index {event_index}"))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No sync event found at the specified index".into(),
                Box::new(event_index),
            )
        })?;

    println!("Sync Event Index {event_index}: {:?}", sync_event);

    Ok(())
}

/// Get summary of L1 manifests in the database.
pub(crate) fn get_sync_events_summary(
    db: Arc<CommonDb>,
    _args: GetSyncEventsSummaryArgs,
) -> Result<(), DisplayedError> {
    // Check sync events present for all L1 blocks
    let sync_db = db.sync_event_db();
    let last_idx = sync_db.get_last_idx().unwrap();

    let l1_db = db.l1_db();
    let (l1_tip_height, _) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    let apparent_genesis_l1_height = (0..=l1_tip_height)
        .rev()
        .find(
            |&height| match l1_db.get_canonical_blockid_at_height(height) {
                Ok(Some(_)) => false, // keep searching
                _ => true,            // break here, found missing or error
            },
        )
        .map(|h| h + 1) // next known good height
        .unwrap_or(l1_tip_height);

    if let Some(last_idx) = last_idx {
        println!(
            "Last sync event index: {}. Expected number of L1 blocks (apparent genesis height to tip): {}",
            last_idx,
            l1_tip_height - apparent_genesis_l1_height + 1
        );
        let mut observed_l1_heights = std::collections::HashSet::new();

        for idx in (1..=last_idx).rev() {
            if let Ok(Some(SyncEvent::L1Block(commitment))) = sync_db.get_sync_event(idx) {
                observed_l1_heights.insert(commitment.height());
            } else {
                println!("Failed to read sync event at index {}", idx);
            }
        }

        // Now verify all expected heights are present
        let all_l1_sync_events_present =
            (apparent_genesis_l1_height..=l1_tip_height).all(|expected_height| {
                if !observed_l1_heights.contains(&expected_height) {
                    println!("Missing SyncEvent::L1Block for height {}", expected_height);
                    return false;
                }
                true
            });

        if all_l1_sync_events_present {
            println!("All expected Sync Events found in SyncEventDatabase.")
        }
    }

    Ok(())
}
