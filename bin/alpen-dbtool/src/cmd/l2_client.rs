use std::sync::Arc;

use clap::Args;
use strata_db::traits::{ClientStateDatabase, Database};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to show details about a specific client state update.
#[derive(Args, Debug)]
pub struct GetL2ClientStateArgs {
    /// Client state update index; defaults to the latest
    state_update_idx: Option<u64>,
}

/// Show details about a specific L2 client state update.
pub fn get_l2_client_state(
    db: Arc<CommonDb>,
    args: GetL2ClientStateArgs,
) -> Result<(), DisplayedError> {
    let last_update_idx = args.state_update_idx.unwrap_or(
        db.client_state_db()
            .get_last_state_idx()
            .internal_error("Failed to fetch last client state index")?,
    );
    let client_state = db
        .client_state_db()
        .get_client_update(last_update_idx)
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at index {last_update_idx}"),
                Box::new(last_update_idx),
            )
        })?;

    println!("Client state index {last_update_idx}");
    println!(
        "client state: genesis_l1_height: {}",
        client_state.state().genesis_l1_height()
    );
    println!(
        "client state: deepest L1 block: {:?}",
        client_state.state().get_deepest_l1_block()
    );
    println!(
        "client state: latest L1 block: {:?}",
        client_state.state().get_tip_l1_block()
    );

    println!(
        "client state: finalized epoch: {:?}",
        client_state.state().get_apparent_finalized_epoch()
    );
    println!(
        "client state: finalized checkpoint: {:?}",
        client_state
            .state()
            .get_apparent_finalized_checkpoint()
            .unwrap()
            .batch_info
    );
    Ok(())
}
