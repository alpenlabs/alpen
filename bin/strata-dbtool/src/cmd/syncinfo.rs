use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{BlockStatus, DatabaseBackend, OLBlockDatabase, OLStateDatabase};
use strata_identifiers::{Buf32, OLBlockId};
use strata_ledger_types::IStateAccessor;
use strata_primitives::l1::L1BlockCommitment;

use super::l1::get_l1_chain_tip;
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

    // Get latest OL state for sync fields.
    let (latest_commitment, top_level_state) = db
        .ol_state_db()
        .get_latest_toplevel_ol_state()
        .internal_error("Failed to get latest OL state")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "OL state not found in database".to_string(),
                Box::new(()),
            )
        })?;

    // OL state does not expose legacy chainstate prev/finalized split. Use the recorded epoch.
    let recorded_epoch = *top_level_state.asm_recorded_epoch();
    let current_epoch = top_level_state.cur_epoch();
    let current_slot = top_level_state.cur_slot();
    let ol_finalized_block_id = OLBlockId::from(Buf32::from(*recorded_epoch.last_blkid()));
    let previous_block = latest_commitment;
    let safe_block = L1BlockCommitment::from_height_u64(
        u64::from(top_level_state.last_l1_height()),
        *top_level_state.last_l1_blkid(),
    )
    .ok_or_else(|| {
        DisplayedError::InternalError(
            "Invalid L1 commitment in OL state".to_string(),
            Box::new(()),
        )
    })?;

    // Create the output data structure
    let sync_info = SyncInfo {
        l1_tip_height,
        l1_tip_block_id: &l1_tip_block_id,
        ol_tip_height,
        ol_tip_block_id: &ol_tip_block_id,
        ol_tip_block_status: &ol_tip_block_status,
        ol_finalized_block_id: &ol_finalized_block_id,
        current_epoch: current_epoch as u64,
        current_slot,
        previous_block: &previous_block,
        previous_epoch: &recorded_epoch,
        finalized_epoch: &recorded_epoch,
        safe_block: &safe_block,
    };

    // Use the output utility
    output(&sync_info, args.output_format)
}

fn get_canonical_ol_block_at_slot(
    db: &impl DatabaseBackend,
    slot: u64,
) -> Result<OLBlockId, DisplayedError> {
    db.ol_block_db()
        .get_blocks_at_height(slot)
        .internal_error("Failed to fetch OL blocks at slot")?
        .first()
        .copied()
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "No OL blocks found at tip slot".to_string(),
                Box::new(slot),
            )
        })
}
