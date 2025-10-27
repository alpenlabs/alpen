use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, Database, L2BlockDatabase};
use strata_primitives::{l1::L1BlockId, l2::L2BlockId};
use strata_state::{block::L2BlockBundle, header::L2Header};

use super::chainstate::get_latest_l2_write_batch;
use crate::{
    cli::OutputFormat,
    output::{
        l2::{L2BlockInfo, L2SummaryInfo},
        output,
    },
    utils::block_id::parse_l2_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l2-block")]
/// Get L2 block
pub(crate) struct GetL2BlockArgs {
    /// L2 Block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l2-summary")]
/// Get L2 summary
pub(crate) struct GetL2SummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get the earliest L2 block id from the database.
pub(crate) fn get_earliest_l2_block_id(
    db: &impl Database,
) -> Result<strata_primitives::l2::L2BlockId, DisplayedError> {
    // Get blocks at slot 0
    let blocks_at_slot_0 = db
        .l2_db()
        .get_blocks_at_height(0)
        .internal_error("Failed to get blocks at slot 0")?;

    if blocks_at_slot_0.is_empty() {
        return Err(DisplayedError::InternalError(
            "No blocks found at slot 0".to_string(),
            Box::new(()),
        ));
    }

    // Return the first block at slot 0
    Ok(blocks_at_slot_0[0])
}

/// Get the slot for a specific L2 block.
pub(crate) fn get_l2_block_slot(
    db: &impl Database,
    block_id: L2BlockId,
) -> Result<Option<u64>, DisplayedError> {
    let Some(block_data) = get_l2_block_data(db, block_id)? else {
        return Ok(None);
    };

    Ok(Some(block_data.block().header().slot()))
}

/// Get both the slot and epoch for a specific L2 block.
pub(crate) fn get_l2_block_slot_and_epoch(
    db: &impl Database,
    block_id: L2BlockId,
) -> Result<Option<(u64, u64)>, DisplayedError> {
    let Some(block_data) = get_l2_block_data(db, block_id)? else {
        return Ok(None);
    };

    let header = block_data.block().header();
    Ok(Some((header.slot(), header.epoch())))
}

/// Get L2 block data by block ID.
pub(crate) fn get_l2_block_data(
    db: &impl Database,
    block_id: L2BlockId,
) -> Result<Option<L2BlockBundle>, DisplayedError> {
    db.l2_db()
        .get_block_data(block_id)
        .internal_error("Failed to read block data")
}

/// Get L2 block by block ID.
pub(crate) fn get_l2_block(db: &impl Database, args: GetL2BlockArgs) -> Result<(), DisplayedError> {
    // Parse block ID using utility function
    let block_id = parse_l2_block_id(&args.block_id)?;

    // Fetch block status and data
    let status = db
        .l2_db()
        .get_block_status(block_id)
        .internal_error("Failed to read block status")?
        .unwrap_or(BlockStatus::Unchecked);

    let bundle = get_l2_block_data(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("L2 block with id not found".to_string(), Box::new(block_id))
    })?;

    let l2_block = bundle.block();
    let header = l2_block.header();
    let l1_segment = l2_block.body().l1_segment();

    // Create L1 segment data
    let l1_segment_data: Vec<(u64, &L1BlockId)> = l1_segment
        .new_manifests()
        .iter()
        .map(|manifest| (manifest.height(), manifest.blkid()))
        .collect();

    // Create the output data structure
    let block_info = L2BlockInfo {
        id: &block_id,
        status: &status,
        header,
        l1_segment: l1_segment_data,
    };

    // Use the output utility
    output(&block_info, args.output_format)
}

/// Get L2 summary - check all L2 blocks exist in database.
pub(crate) fn get_l2_summary(
    db: &impl Database,
    args: GetL2SummaryArgs,
) -> Result<(), DisplayedError> {
    // Get the tip block (highest slot)
    let latest_write_batch = get_latest_l2_write_batch(db)
        .internal_error("Failed to get latest write batch")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Failed to get latest write batch".to_string(),
                Box::new(()),
            )
        })?;
    let top_level_state = latest_write_batch.toplevel_chainstate();
    let tip_slot = top_level_state.chain_tip_slot();
    let tip_block_id = latest_write_batch.blockid();
    let current_epoch = top_level_state.cur_epoch();

    // Get the earliest block (slot 0)
    let earliest_block_id = get_earliest_l2_block_id(db)?;
    let earliest_block_data = get_l2_block_data(db, earliest_block_id)?.ok_or_else(|| {
        DisplayedError::InternalError(
            "L2 block data not found in database".to_string(),
            Box::new(earliest_block_id),
        )
    })?;
    let earliest_slot = earliest_block_data.block().header().slot();

    // Check for gaps between earliest and tip slots
    let mut missing_slots = Vec::new();
    for slot in earliest_slot..=tip_slot {
        let blocks_at_slot = db
            .l2_db()
            .get_blocks_at_height(slot)
            .internal_error(format!("Failed to get blocks at height {slot}"))?;

        if blocks_at_slot.is_empty() {
            missing_slots.push(slot);
        }
    }

    let expected_block_count = tip_slot.saturating_sub(earliest_slot) + 1;
    let all_blocks_present = missing_slots.is_empty();

    // Create the output data structure
    let summary_info = L2SummaryInfo {
        tip_slot,
        tip_block_id,
        earliest_slot,
        earliest_block_id: &earliest_block_id,
        current_epoch,
        expected_block_count,
        all_blocks_present,
        missing_slots,
    };

    // Use the output utility
    output(&summary_info, args.output_format)
}
