use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{BlockStatus, DatabaseBackend, OLBlockDatabase, OLStateDatabase};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};
use strata_primitives::l1::L1BlockCommitment;

use super::{
    checkpoint::{
        get_canonical_epoch_commitment_at, get_checkpoint_status_at_epoch,
        get_latest_finalized_checkpoint_epoch,
    },
    l1::get_l1_chain_tip,
    ol::get_canonical_ol_block_at_slot,
};
use crate::{
    cli::OutputFormat,
    output::{output, syncinfo::SyncInfo},
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-syncinfo")]
/// Get sync info
pub(crate) struct GetSyncinfoArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,

    /// L1 reorg-safe depth used to derive finalized checkpoint epoch
    #[argh(option)]
    pub(crate) l1_reorg_safe_depth: u32,
}

/// Show the latest sync information.
pub(crate) fn get_syncinfo(
    db: &impl DatabaseBackend,
    args: GetSyncinfoArgs,
) -> Result<(), DisplayedError> {
    // Get L1 tip
    let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

    // Get OL tip slot and select canonical tip block using first block at that slot.
    let ol_tip_height = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;
    let ol_tip_block_id = get_canonical_ol_block_at_slot(db, ol_tip_height)?;

    // Get OL tip block status from OL block db.
    let ol_tip_block_status = db
        .ol_block_db()
        .get_block_status(ol_tip_block_id)
        .internal_error("Failed to get OL tip block status")?
        .unwrap_or(BlockStatus::Unchecked);
    let ol_tip_block = db
        .ol_block_db()
        .get_block_data(ol_tip_block_id)
        .internal_error("Failed to get OL tip block data")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "OL tip block data not found in database".to_string(),
                Box::new(ol_tip_block_id),
            )
        })?;

    // Use the same chosen canonical tip commitment for OL state reads.
    let ol_tip_commitment = OLBlockCommitment::new(ol_tip_height, ol_tip_block_id);
    let top_level_state = db
        .ol_state_db()
        .get_toplevel_ol_state(ol_tip_commitment)
        .internal_error("Failed to get OL state at canonical tip commitment")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "OL state not found for canonical tip commitment".to_string(),
                Box::new(ol_tip_commitment),
            )
        })?;

    let current_epoch = top_level_state.epoch_state().cur_epoch();
    let previous_epoch_num = current_epoch.saturating_sub(1);
    let previous_epoch = if previous_epoch_num == 0 {
        EpochCommitment::null()
    } else {
        get_canonical_epoch_commitment_at(db, previous_epoch_num)?.ok_or_else(|| {
            DisplayedError::InternalError(
                "Previous epoch commitment missing in OL checkpoint DB".to_string(),
                Box::new(previous_epoch_num),
            )
        })?
    };
    let previous_epoch_status =
        get_checkpoint_status_at_epoch(db, previous_epoch_num, args.l1_reorg_safe_depth)?
            .map(|status| status.as_str().to_string());
    let finalized_epoch = get_latest_finalized_checkpoint_epoch(db, args.l1_reorg_safe_depth)?
        .unwrap_or_else(EpochCommitment::null);
    let current_slot = top_level_state.global_state().get_cur_slot();
    let ol_finalized_block_id = *finalized_epoch.last_blkid();
    let previous_block = OLBlockCommitment::new(
        ol_tip_block.header().slot().saturating_sub(1),
        *ol_tip_block.header().parent_blkid(),
    );
    let safe_block = L1BlockCommitment::new(
        top_level_state.epoch_state().last_l1_height(),
        *top_level_state.epoch_state().last_l1_blkid(),
    );

    // Create the output data structure
    let sync_info = SyncInfo {
        l1_tip_height,
        l1_tip_block_id: &l1_tip_block_id,
        ol_tip_height,
        ol_tip_block_id: &ol_tip_block_id,
        ol_tip_block_status: &ol_tip_block_status,
        ol_finalized_block_id: &ol_finalized_block_id,
        current_epoch,
        current_slot,
        previous_block: &previous_block,
        previous_epoch: &previous_epoch,
        previous_epoch_status,
        finalized_epoch: &finalized_epoch,
        safe_block: &safe_block,
    };

    // Use the output utility
    output(&sync_info, args.output_format)
}
