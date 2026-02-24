//! OL genesis initialization for the new strata binary.

use anyhow::Result;
use strata_db_types::traits::BlockStatus;
use strata_ol_genesis::{GenesisArtifacts, build_genesis_artifacts};
use strata_ol_params::OLParams;
use strata_primitives::OLBlockCommitment;
use strata_storage::NodeStorage;
use tracing::{info, instrument};

/// Initialize the OL genesis block and state for a fresh database.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub(crate) fn init_ol_genesis(
    ol_params: &OLParams,
    storage: &NodeStorage,
) -> Result<OLBlockCommitment> {
    info!("initializing OL genesis block and state");

    let GenesisArtifacts {
        ol_state,
        ol_block,
        commitment,
        epoch_summary,
    } = build_genesis_artifacts(ol_params)?;
    let genesis_blkid = *commitment.blkid();

    // Insert creation epoch 0 for all genesis accounts.
    ol_state.ledger.accounts.iter().try_for_each(|entry| {
        info!(%entry.id, "inserting account info");
        storage
            .account()
            .insert_account_creation_epoch_blocking(entry.id, 0)
    })?;

    storage.ol_block().put_block_data_blocking(ol_block)?;
    storage
        .ol_block()
        .set_block_status_blocking(genesis_blkid, BlockStatus::Valid)?;

    // Store genesis OL state
    storage
        .ol_state()
        .put_toplevel_ol_state_blocking(commitment, ol_state)?;

    storage
        .ol_checkpoint()
        .insert_epoch_summary_blocking(epoch_summary)?;

    info!(%genesis_blkid, slot = 0, "OL genesis initialization complete");
    Ok(commitment)
}
