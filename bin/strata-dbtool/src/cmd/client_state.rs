use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ClientStateDatabase, Database};
use strata_state::{
    client_state::ClientState,
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
    client_state: &'a ClientState,
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
            client_state: &client_state,
            sync_actions: &sync_actions,
        };
        println!("{}", serde_json::to_string_pretty(&update_info).unwrap());
    } else {
        println!("Client state update index {update_idx}");
        println!("Client state {client_state:?}");
        println!("Sync actions {sync_actions:?}");
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
