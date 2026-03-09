use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{BlockStatus, DatabaseBackend, OLBlockDatabase, OLStateDatabase};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};
use strata_ledger_types::IStateAccessor;
use strata_primitives::l1::L1BlockCommitment;

use super::{
    client_state::get_declared_final_epoch, l1::get_l1_chain_tip,
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

    // OL state exposes ASM-recorded epoch for previous epoch fields.
    let recorded_epoch = *top_level_state.asm_recorded_epoch();
    // Finalized epoch should come from client-state declared final epoch (L1-confirmed).
    let finalized_epoch = get_declared_final_epoch(db)?.unwrap_or_else(EpochCommitment::null);
    let current_epoch = top_level_state.cur_epoch();
    let current_slot = top_level_state.cur_slot();
    let ol_finalized_block_id = *finalized_epoch.last_blkid();
    let previous_block = OLBlockCommitment::new(
        ol_tip_block.header().slot().saturating_sub(1),
        *ol_tip_block.header().parent_blkid(),
    );
    let safe_block = L1BlockCommitment::new(
        top_level_state.last_l1_height(),
        *top_level_state.last_l1_blkid(),
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
        previous_epoch: &recorded_epoch,
        finalized_epoch: &finalized_epoch,
        safe_block: &safe_block,
    };

    // Use the output utility
    output(&sync_info, args.output_format)
}
