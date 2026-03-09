use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{
    BlockStatus, DatabaseBackend, OLBlockDatabase, OLCheckpointDatabase,
};
use strata_identifiers::{Epoch, OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::l1::L1BlockId;

use crate::{
    cli::OutputFormat,
    output::{
        ol::{OLBlockInfo, OLSummaryInfo},
        output,
    },
    utils::block_id::parse_ol_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-ol-block")]
/// Get OL block.
pub(crate) struct GetOLBlockArgs {
    /// OL block id (hex)
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-ol-summary")]
/// Get OL block summary.
pub(crate) struct GetOLSummaryArgs {
    /// slot to start scanning OL summary from
    #[argh(positional)]
    pub(crate) slot_from: Slot,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get OL block by block ID.
pub(crate) fn get_ol_block(
    db: &impl DatabaseBackend,
    args: GetOLBlockArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_ol_block_id(&args.block_id)?;

    // Fetch block status and data from OL block database.
    let status = db
        .ol_block_db()
        .get_block_status(block_id)
        .internal_error("Failed to read block status")?
        .unwrap_or(BlockStatus::Unchecked);

    let ol_block = get_ol_block_data(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
    })?;

    let header = ol_block.header();

    // Create manifest data from OL terminal block update (if present).
    let manifest_data: Vec<(u64, &L1BlockId)> = if let Some(update) = ol_block.body().l1_update() {
        update
            .manifest_cont()
            .manifests()
            .iter()
            .map(|manifest| (u64::from(manifest.height()), manifest.blkid()))
            .collect()
    } else {
        Vec::new()
    };

    // Create the output data structure
    let block_info = OLBlockInfo {
        id: &block_id,
        status: &status,
        header_slot: header.slot(),
        header_epoch: header.epoch(),
        header_timestamp: header.timestamp(),
        header_prev_blkid: *header.parent_blkid(),
        header_body_root: *header.body_root(),
        header_logs_root: *header.logs_root(),
        header_state_root: *header.state_root(),
        manifests: manifest_data,
    };

    // Use the output utility
    output(&block_info, args.output_format)
}

/// Get OL block summary - check all OL blocks exist in database.
pub(crate) fn get_ol_summary(
    db: &impl DatabaseBackend,
    args: GetOLSummaryArgs,
) -> Result<(), DisplayedError> {
    // Get the tip block (highest slot) from OL block database.
    let tip_block_id = get_chain_tip_ol_block_id(db)?;
    let tip_block_data = get_ol_block_data(db, tip_block_id)?.ok_or_else(|| {
        DisplayedError::InternalError(
            "OL block data not found in database".to_string(),
            Box::new(tip_block_id),
        )
    })?;
    let tip_slot = tip_block_data.header().slot();

    let from_slot = args.slot_from;
    if from_slot > tip_slot {
        return Err(DisplayedError::UserError(
            "slot_from is after OL tip slot".to_string(),
            Box::new(from_slot),
        ));
    }
    let from_block_id = get_canonical_ol_block_at_slot(db, from_slot)?;

    // Check for gaps between from slot and tip slot.
    let mut missing_slots = Vec::new();
    for slot in from_slot..=tip_slot {
        let blocks_at_slot = db
            .ol_block_db()
            .get_blocks_at_height(slot)
            .internal_error(format!("Failed to get blocks at height {slot}"))?;

        if blocks_at_slot.is_empty() {
            missing_slots.push(slot);
        }
    }

    let expected_block_count = tip_slot.saturating_sub(from_slot) + 1;
    let all_blocks_present = missing_slots.is_empty();

    // Get last epoch from OL checkpoint database.
    let last_epoch = get_last_ol_checkpoint_epoch(db)?;

    // Create the output data structure
    let summary_info = OLSummaryInfo {
        tip_slot,
        tip_block_id: &tip_block_id,
        from_slot,
        from_block_id: &from_block_id,
        last_epoch,
        expected_block_count,
        all_blocks_present,
        missing_slots,
    };

    // Use the output utility
    output(&summary_info, args.output_format)
}

fn get_chain_tip_ol_block_id(db: &impl DatabaseBackend) -> Result<OLBlockId, DisplayedError> {
    let tip_slot = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;

    get_canonical_ol_block_at_slot(db, tip_slot)
}

pub(crate) fn get_canonical_ol_block_at_slot(
    db: &impl DatabaseBackend,
    slot: Slot,
) -> Result<OLBlockId, DisplayedError> {
    let blocks = db
        .ol_block_db()
        .get_blocks_at_height(slot)
        .internal_error("Failed to fetch OL blocks at slot")?;

    blocks.first().copied().ok_or_else(|| {
        DisplayedError::InternalError("No OL blocks found at slot".to_string(), Box::new(slot))
    })
}

fn get_ol_block_data(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<Option<OLBlock>, DisplayedError> {
    db.ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")
}

fn get_last_ol_checkpoint_epoch(
    db: &impl DatabaseBackend,
) -> Result<Option<Epoch>, DisplayedError> {
    db.ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last OL checkpoint epoch")
}
