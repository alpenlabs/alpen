use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ClientStateDatabase, Database};
use strata_primitives::prelude::L1BlockCommitment;
use strata_state::{
    client_state::InternalState,
    l1::L1BlockId,
    operation::{ClientUpdateOutput, SyncAction},
};

use crate::cli::OutputFormat;

/// Shows details about a client state update
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-client-state-update")]
pub(crate) struct GetClientStateUpdateArgs {
    /// client state update index; defaults to the latest
    #[argh(positional)]
    pub(crate) state_update_idx: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Strata client state update displayed to the user
#[derive(serde::Serialize)]
struct ClientStateUpdateInfo<'a> {
    update_index: u64,
    is_chain_active: bool,
    horizon_l1_height: u64,
    genesis_l1_height: u64,
    latest_l1_block: Option<&'a L1BlockId>,
    next_expected_l1_height: u64,
    tip_l1_block: Option<L1BlockCommitment>,
    deepest_l1_block: Option<L1BlockCommitment>,
    last_internal_state: Option<&'a InternalState>,
    sync_actions: &'a Vec<SyncAction>,
}

/// Show details about a specific L2 client state update.
pub(crate) fn get_client_state_update(
    db: &impl Database,
    args: GetClientStateUpdateArgs,
) -> Result<(), DisplayedError> {
    let (client_state_update, update_idx) =
        get_latest_client_state_update(db, args.state_update_idx)?;
    let (client_state, sync_actions) = client_state_update.into_parts();

    if args.output_format == OutputFormat::Json {
        let update_info = ClientStateUpdateInfo {
            update_index: update_idx,
            is_chain_active: client_state.is_chain_active(),
            horizon_l1_height: client_state.horizon_l1_height(),
            genesis_l1_height: client_state.genesis_l1_height(),
            latest_l1_block: client_state.most_recent_l1_block(),
            next_expected_l1_height: client_state.next_exp_l1_block(),
            tip_l1_block: client_state.get_tip_l1_block(),
            deepest_l1_block: client_state.get_deepest_l1_block(),
            last_internal_state: client_state.get_last_internal_state(),
            sync_actions: &sync_actions,
        };
        println!("{}", serde_json::to_string_pretty(&update_info).unwrap());
    } else {
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
                SyncAction::FinalizeEpoch(epoch) => {
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
                SyncAction::L2Genesis(block_id) => {
                    println!("client_state_update.sync_action L2Genesis");
                    println!("client_state_update.sync_action.blkid {block_id:?}");
                }
                SyncAction::UpdateCheckpointInclusion { .. } => {
                    println!("client_state_update.sync_action UpdateCheckpointInclusion");
                }
            }
        }
    }

    Ok(())
}

/// Get the latest client state update from the database.
pub(super) fn get_latest_client_state_update(
    db: &impl Database,
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
