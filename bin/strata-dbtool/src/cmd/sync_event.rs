use std::collections::HashSet;

use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{DatabaseBackend, SyncEventDatabase};
use strata_state::sync_event::SyncEvent;
use tracing::warn;

use super::l1::get_l1_chain_tip;
use crate::{
    cli::OutputFormat,
    cmd::client_state::get_latest_client_state_update,
    output::{
        output,
        sync_event::{SyncEventInfo, SyncEventsSummaryInfo},
    },
};

/// Shows details about a sync event
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-sync-event")]
pub(crate) struct GetSyncEventArgs {
    /// sync event index
    #[argh(positional)]
    pub(crate) event_index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Shows a summary of all sync events
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-sync-events-summary")]
pub(crate) struct GetSyncEventsSummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get SyncEvent details by index.
/// Get SyncEvent details by index.
pub(crate) fn get_sync_event(
    db: &impl DatabaseBackend,
    args: GetSyncEventArgs,
) -> Result<(), DisplayedError> {
    let sync_db = db.sync_event_db();
    let event_index = args.event_index;

    let sync_event = sync_db
        .get_sync_event(event_index)
        .internal_error(format!("Failed to get sync event at index {event_index}"))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No sync event found at the specified index".into(),
                Box::new(event_index),
            )
        })?;

    // Create the output data structure
    let event_info = SyncEventInfo {
        event_index,
        event: &sync_event,
    };

    // Use the output utility
    output(&event_info, args.output_format)
}

/// Get summary of L1 manifests in the database.
pub(crate) fn get_sync_events_summary(
    db: &impl DatabaseBackend,
    args: GetSyncEventsSummaryArgs,
) -> Result<(), DisplayedError> {
    // Check sync events present for all L1 blocks
    let sync_db = db.sync_event_db();
    let last_idx = sync_db.get_last_idx().unwrap();

    // Use helper function to get L1 tip
    let (l1_tip_height, _) = get_l1_chain_tip(db)?;

    let (client_state_update, _) = get_latest_client_state_update(db, None)?;
    let (client_state, _) = client_state_update.into_parts();
    let horizon_l1_height = client_state.horizon_l1_height();

    if horizon_l1_height == l1_tip_height {
        warn!("Missing all l1 blocks from horizon to tip.");
    }

    let mut missing_heights = Vec::new();
    let mut all_sync_events_in_db = true;

    if let Some(last_idx) = last_idx {
        let mut observed_l1_heights = HashSet::new();

        for idx in (1..=last_idx).rev() {
            if let Ok(Some(SyncEvent::L1Block(commitment))) = sync_db.get_sync_event(idx) {
                observed_l1_heights.insert(commitment.height());
            } else {
                println!("Failed to read sync event at index {idx}");
            }
        }

        // Now verify all expected heights are present
        for expected_height in horizon_l1_height..=l1_tip_height {
            if !observed_l1_heights.contains(&expected_height) {
                missing_heights.push(expected_height);
                all_sync_events_in_db = false;
            }
        }
    }

    let output_data = SyncEventsSummaryInfo {
        last_event_index: last_idx,
        expected_l1_blocks_count: l1_tip_height.saturating_sub(horizon_l1_height) + 1,
        all_sync_events_in_db,
        missing_heights,
    };

    output(&output_data, args.output_format)
}
