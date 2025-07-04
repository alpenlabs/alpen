use argh::FromArgs;
use strata_db::traits::{ClientStateDatabase, Database};
use strata_state::operation::{ClientUpdateOutput, SyncAction};

use crate::{
    cli::OutputFormat,
    errors::{DisplayableError, DisplayedError},
};

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

/// Show details about a specific L2 client state update.
pub(crate) fn get_client_state_update(
    db: &impl Database,
    args: GetClientStateUpdateArgs,
) -> Result<(), DisplayedError> {
    let (client_state_update, update_idx) =
        get_latest_client_state_update(db, args.state_update_idx)?;
    let (client_state, sync_actions) = client_state_update.into_parts();

    println!("Client state index {update_idx}");
    println!(
        "client state: genesis l1 height: {}",
        client_state.genesis_l1_height()
    );
    println!(
        "client state: deepest L1 block: {:?}",
        client_state.get_deepest_l1_block()
    );
    println!(
        "client state: latest L1 block: {:?}",
        client_state.get_tip_l1_block()
    );

    println!(
        "client state: finalized epoch: {:?}",
        client_state.get_apparent_finalized_epoch()
    );
    println!(
        "client state: finalized checkpoint: {:?}",
        client_state
            .get_apparent_finalized_checkpoint()
            .unwrap()
            .batch_info
    );

    for act in sync_actions {
        match act {
            SyncAction::FinalizeEpoch(epoch_commitment) => {
                println!("client state: sync action: epoch commitment {epoch_commitment:?}");
            }
            SyncAction::L2Genesis(l2_block_id) => {
                println!("client state: sync action: L2 block {l2_block_id:?}");
            }
            SyncAction::UpdateCheckpointInclusion {
                checkpoint,
                l1_reference,
            } => {
                println!(
                    "client state sync action: Update checkpoint inclusion for: {:?}",
                    checkpoint.commitment()
                );
                println!(
                    "client state sync action: Update checkpoint inclusion with l1 reference {l1_reference:?}"
                );
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
