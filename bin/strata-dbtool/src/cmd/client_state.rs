use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_csm_types::ClientUpdateOutput;
use strata_db::traits::{ClientStateDatabase, DatabaseBackend};
use strata_primitives::l1::L1BlockCommitment;

use crate::{
    cli::OutputFormat,
    cmd::l1::get_l1_block_manifest,
    output::{client_state::ClientStateUpdateInfo, output},
    utils::block_id::parse_l1_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-client-state-update")]
/// Get client state update for a given L1 block
pub(crate) struct GetClientStateUpdateArgs {
    /// L1 block ID (hex string)
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get client state update for the specified L1 block.
pub(crate) fn get_client_state_update(
    db: &impl DatabaseBackend,
    args: GetClientStateUpdateArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_l1_block_id(&args.block_id)?;
    let block_mf = get_l1_block_manifest(db, block_id)?;
    let block_commitment: L1BlockCommitment = block_mf
        .ok_or(DisplayedError::InternalError("".to_string(), Box::new(())))?
        .into();

    let (client_state, actions) = db
        .client_state_db()
        .get_client_update(block_commitment)
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at block {block_commitment}"),
                Box::new(()),
            )
        })?
        .into_parts();

    // Create the output data structure
    let client_state_info = ClientStateUpdateInfo {
        state: client_state,
        sync_actions: &actions,
        block: block_commitment,
    };

    // Use the output utility
    output(&client_state_info, args.output_format)
}

/// Get the latest client state update from the database.
pub(crate) fn _get_latest_client_state_update(
    db: &impl DatabaseBackend,
) -> Result<(ClientUpdateOutput, L1BlockCommitment), DisplayedError> {
    let client_state_db = db.client_state_db();

    let (latest_block, _) = client_state_db
        .get_latest_client_state()
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::InternalError("No client state found".to_string(), Box::new(()))
        })?;

    let client_state = client_state_db
        .get_client_update(latest_block)
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at index {latest_block}"),
                Box::new(()),
            )
        })?;

    Ok((client_state, latest_block))
}
