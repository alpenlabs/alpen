use std::collections::HashSet;

use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{Database, SyncEventDatabase};
use strata_state::sync_event::SyncEvent;
use tracing::warn;

use super::l1::get_l1_chain_tip;
use crate::{cli::OutputFormat, cmd::client_state::get_latest_client_state_update};

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
pub(crate) fn get_sync_event(
    db: &impl Database,
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

    // Print in porcelain format
    println!("sync_event.event_index {event_index}");

    match &sync_event {
        SyncEvent::L1Block(l1_commitment) => {
            println!("sync_event.event L1Block");
            println!(
                "sync_event.event.l1_block_commitment.height {}",
                l1_commitment.height()
            );
            println!(
                "sync_event.event.l1_block_commitment.blkid {:?}",
                l1_commitment.blkid()
            );
        }
        SyncEvent::L1Revert(l1_commitment) => {
            println!("sync_event.event L1Revert");
            println!(
                "sync_event.event.l1_block_commitment.height {}",
                l1_commitment.height()
            );
            println!(
                "sync_event.event.l1_block_commitment.blkid {:?}",
                l1_commitment.blkid()
            );
        }
    }

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

    // Print in porcelain format
    if let Some(last_idx) = last_idx {
        println!("sync_events_summary.last_event_index {last_idx}");
    }

    println!(
        "sync_events_summary.expected_l1_blocks_count {}",
        l1_tip_height.saturating_sub(horizon_l1_height) + 1
    );
    println!(
        "sync_events_summary.all_sync_events_in_db {}",
        if all_sync_events_in_db {
            "true"
        } else {
            "false"
        }
    );

    // Add missing heights if any
    for height in &missing_heights {
        println!("Missing SyncEvent::L1Block for height {height}");
    }

    Ok(())
}
