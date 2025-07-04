use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{Database, L1Database, SyncEventDatabase};
use strata_state::sync_event::SyncEvent;
use tracing::warn;

use super::l1::get_l1_horizon_height;
use crate::cli::OutputFormat;

/// Shows details about a sync event
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-sync-event")]
pub(crate) struct GetSyncEventArgs {
    /// sync event index; defaults to the last written index
    #[argh(positional)]
    pub(crate) event_index: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Shows a summary of all sync events
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-sync-events-summary")]
pub(crate) struct GetSyncEventsSummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get SyncEvent details by index.
pub(crate) fn get_sync_event(
    db: &impl Database,
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

    println!("Sync Event Index {event_index}: {sync_event:?}");

    Ok(())
}

/// Get summary of L1 manifests in the database.
pub(crate) fn get_sync_events_summary(
    db: &impl Database,
    _args: GetSyncEventsSummaryArgs,
) -> Result<(), DisplayedError> {
    // Check sync events present for all L1 blocks
    let sync_db = db.sync_event_db();
    let last_idx = sync_db.get_last_idx().unwrap();

    let (l1_tip_height, _) = db
        .l1_db()
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    let l1_horizon_height = get_l1_horizon_height(db, l1_tip_height);
    if l1_horizon_height == l1_tip_height {
        warn!("Missing all l1 blocks from horizon to tip.");
    }

    if let Some(last_idx) = last_idx {
        println!(
            "Last sync event index: {}. Expected number of L1 blocks (l1 horizon to tip): {}",
            last_idx,
            l1_tip_height - l1_horizon_height + 1
        );
        let mut observed_l1_heights = std::collections::HashSet::new();

        for idx in (1..=last_idx).rev() {
            if let Ok(Some(SyncEvent::L1Block(commitment))) = sync_db.get_sync_event(idx) {
                observed_l1_heights.insert(commitment.height());
            } else {
                println!("Failed to read sync event at index {idx}");
            }
        }

        // Now verify all expected heights are present
        let all_l1_sync_events_present =
            (l1_horizon_height..=l1_tip_height).all(|expected_height| {
                if !observed_l1_heights.contains(&expected_height) {
                    println!("Missing SyncEvent::L1Block for height {expected_height}");
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
