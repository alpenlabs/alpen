use std::sync::Arc;

use clap::Args;
use strata_db::traits::{ClientStateDatabase, Database};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

#[derive(Args, Debug)]
pub struct GetL2ClientStateArgs {
    /// Client state update index; defaults to the latest
    state_update_idx: Option<u64>,
}

pub fn get_l2_client_state(
    db: Arc<CommonDb>,
    args: GetL2ClientStateArgs,
) -> Result<(), DisplayedError> {
    let client_state_idx = args.state_update_idx.unwrap_or(
        db.client_state_db()
            .get_last_state_idx()
            .internal_error("Failed to fetch last client state index")?,
    );
    let client_state = db
        .client_state_db()
        .get_client_update(client_state_idx)
        .internal_error("Failed to fetch client state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No client state found at index {client_state_idx}"),
                Box::new(client_state_idx),
            )
        })?;

    println!("Client state index {client_state_idx}");
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
