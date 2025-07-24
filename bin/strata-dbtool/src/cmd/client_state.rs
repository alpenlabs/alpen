use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ClientStateDatabase, DatabaseBackend};
use strata_state::operation::ClientUpdateOutput;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-client-state-update")]
/// Get client state update
pub(crate) struct GetClientStateUpdateArgs {
    /// client state update index; defaults to the latest
    #[argh(positional)]
    pub(crate) update_index: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get client state update at specified index.
pub(crate) fn get_client_state_update(
    db: &impl DatabaseBackend,
    args: GetClientStateUpdateArgs,
) -> Result<(), DisplayedError> {
    let (client_state_update, update_idx) = get_latest_client_state_update(db, args.update_index)?;
    let (client_state, sync_actions) = client_state_update.into_parts();

    // Print in porcelain format
    println!("client_state_update.update_index {update_idx}");
    println!(
        "client_state_update.client_state.is_chain_active {}",
        client_state.is_chain_active()
    );
    println!(
        "client_state_update.client_state.horizon_l1_height {}",
        client_state.horizon_l1_height()
    );
    println!(
        "client_state_update.client_state.genesis_l1_height {}",
        client_state.genesis_l1_height()
    );
    if let Some(l1_block) = client_state.most_recent_l1_block() {
        println!("client_state_update.client_state.latest_l1_block {l1_block:?}");
    }
    println!(
        "client_state_update.client_state.next_expected_l1_height {}",
        client_state.next_exp_l1_block()
    );

    if let Some(tip_l1_block) = client_state.get_tip_l1_block() {
        println!(
            "client_state_update.client_state.tip_l1_block.height {}",
            tip_l1_block.height()
        );
        println!(
            "client_state_update.client_state.tip_l1_block.blkid {:?}",
            tip_l1_block.blkid()
        );
    }

    if let Some(tip_l1_block) = client_state.get_deepest_l1_block() {
        println!(
            "client_state_update.client_state.deepest_l1_block.height {}",
            tip_l1_block.height()
        );
        println!(
            "client_state_update.client_state.deepest_l1_block.blkid {:?}",
            tip_l1_block.blkid()
        );
    }

    if let Some(last_internal_state) = client_state.get_last_internal_state() {
        println!(
            "client_state_update.client_state.last_internal_state.blkid {:?}",
            last_internal_state.blkid()
        );
    }

    for sync_action in sync_actions.iter() {
        match sync_action {
            strata_state::operation::SyncAction::FinalizeEpoch(epoch) => {
                println!("client_state_update.sync_action FinalizeEpoch");
                println!("client_state_update.sync_action.epoch {}", epoch.epoch());
                println!(
                    "client_state_update.sync_action.last_slot {}",
                    epoch.last_slot()
                );
                println!(
                    "client_state_update.sync_action.last_blkid {:?}",
                    epoch.last_blkid()
                );
            }
            strata_state::operation::SyncAction::L2Genesis(block_id) => {
                println!("client_state_update.sync_action L2Genesis");
                println!("client_state_update.sync_action.blkid {block_id:?}");
            }
            strata_state::operation::SyncAction::UpdateCheckpointInclusion { .. } => {
                println!("client_state_update.sync_action UpdateCheckpointInclusion");
            }
        }
    }

    Ok(())
}

/// Get the latest client state update from the database.
pub(crate) fn get_latest_client_state_update(
    db: &impl DatabaseBackend,
    update_idx: Option<u64>,
) -> Result<(ClientUpdateOutput, u64), DisplayedError> {
    let client_state_db = db.client_state_db();
    let last_update_idx = update_idx.unwrap_or(
        client_state_db
            .get_last_state_idx()
            .internal_error("Failed to fetch last client state index")?,
    );
    let client_state = client_state_db
        .get_client_update(last_update_idx)
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at index {last_update_idx}"),
                Box::new(last_update_idx),
            )
        })?;

    Ok((client_state, last_update_idx))
}
