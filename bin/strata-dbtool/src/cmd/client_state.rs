use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ClientStateDatabase, DatabaseBackend};
use strata_primitives::l1::L1BlockCommitment;
use strata_state::operation::ClientUpdateOutput;

use crate::{
    cli::OutputFormat,
    output::{output, traits::FmtStub},
};

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

    // Create the output data structure
    //let client_state_info = ClientStateUpdateInfo {
    //    update_index: update_idx,
    //    is_chain_active: client_state.is_chain_active(),
    //    horizon_l1_height: client_state.horizon_l1_height(),
    //    genesis_l1_height: client_state.genesis_l1_height(),
    //    latest_l1_block: client_state.most_recent_l1_block(),
    //    next_expected_l1_height: client_state.next_exp_l1_block(),
    //    tip_l1_block: client_state.get_tip_l1_block(),
    //    deepest_l1_block: client_state.get_deepest_l1_block(),
    //    last_internal_state: client_state.get_last_internal_state(),
    //    sync_actions: &sync_actions,
    //};
    let client_state_info = FmtStub {};

    // Use the output utility
    output(&client_state_info, args.output_format)
}

/// Get the latest client state update from the database.
pub(crate) fn get_latest_client_state_update(
    db: &impl DatabaseBackend,
    update_idx: Option<u64>,
) -> Result<(ClientUpdateOutput, u64), DisplayedError> {
    //TODO(QQ):
    let client_state_db = db.client_state_db();
    let last_update_idx = 0;
    //update_idx.unwrap_or(
    //    client_state_db
    //        .get_last_state_idx()
    //        .internal_error("Failed to fetch last client state index")?,
    //);

    let client_state = client_state_db
        .get_client_update(L1BlockCommitment::default())
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at index {last_update_idx}"),
                Box::new(last_update_idx),
            )
        })?;

    Ok((client_state, last_update_idx))
}
