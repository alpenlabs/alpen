use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{Database, L1Database, SyncEventDatabase};
use strata_primitives::prelude::L1BlockCommitment;
use strata_state::sync_event::SyncEvent;
use tracing::warn;

use super::client_state::get_latest_client_state_update;
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
    pub(crate) _output_format: OutputFormat,
}

/// Sync event information displayed to the user
#[derive(serde::Serialize)]
struct SyncEventInfo<'a> {
    event_index: u64,
    event: &'a SyncEvent,
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

    // Print sync event information
    if args.output_format == OutputFormat::Json {
        let event_info = SyncEventInfo {
            event_index,
            event: &sync_event,
        };
        println!("{}", serde_json::to_string_pretty(&event_info).unwrap());
    } else {
        println!("sync_event.event_index {event_index}");
        match sync_event {
            SyncEvent::L1Block(ref l1_commitment) => {
                println!("sync_event.event L1Block");
                print_l1_block_commitment(l1_commitment);
            }
            SyncEvent::L1Revert(ref l1_commitment) => {
                println!("sync_event.event L1Revert");
                print_l1_block_commitment(l1_commitment);
            }
        }
    }

    Ok(())
}

/// Print L1 block commitment for a sync event
fn print_l1_block_commitment(l1_ref: &L1BlockCommitment) {
    println!(
        "sync_event.event.l1_block_commitment.height {}",
        l1_ref.height()
    );
    println!(
        "sync_event.event.l1_block_commitment.blkid {:?}",
        l1_ref.blkid()
    );
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

    let (client_state_update, _) = get_latest_client_state_update(db, None)?;
    let (client_state, _) = client_state_update.into_parts();
    let horizon_l1_height = client_state.horizon_l1_height();

    if horizon_l1_height == l1_tip_height {
        warn!("Missing all l1 blocks from horizon to tip.");
    }

    if let Some(last_idx) = last_idx {
        println!("sync_events_summary.last_event_index {last_idx}");
        println!(
            "sync_events_summary.expected_l1_blocks_count {}",
            l1_tip_height.saturating_sub(horizon_l1_height) + 1
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
            (horizon_l1_height..=l1_tip_height).all(|expected_height| {
                if !observed_l1_heights.contains(&expected_height) {
                    println!("Missing SyncEvent::L1Block for height {expected_height}");
                    return false;
                }
                true
            });

        println!("sync_events_summary.all_sync_events_in_db {all_l1_sync_events_present}");
    }

    Ok(())
}
