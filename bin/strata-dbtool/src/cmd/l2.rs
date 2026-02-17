use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
use strata_db_types::traits::{BlockStatus, DatabaseBackend, L2BlockDatabase};
use strata_ol_chain_types::{L2BlockBundle, L2Header};
use strata_primitives::{l1::L1BlockId, l2::L2BlockId};

use super::checkpoint::get_last_epoch;
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

/// Get the [`L2BlockId`] of the chain tip from the database.
pub(crate) fn get_chain_tip_block_id(
    db: &impl DatabaseBackend,
) -> Result<L2BlockId, DisplayedError> {
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    db.l2_db()
        .get_tip_block()
        .internal_error("Failed to get chain tip block")
}

/// Get the [`L2BlockId`] of the earliest L2 block from the database.
pub(crate) fn get_earliest_l2_block_id(
    db: &impl DatabaseBackend,
) -> Result<L2BlockId, DisplayedError> {
    // Get blocks at slot 0
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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

/// Get slot and epoch for a specific L2 block.
pub(crate) fn get_l2_block_slot_and_epoch(
    db: &impl DatabaseBackend,
    block_id: L2BlockId,
) -> Result<Option<(u64, u64)>, DisplayedError> {
    let Some(block_data) = get_l2_block_data(db, block_id)? else {
        return Ok(None);
    };

    let header = block_data.block().header();
    Ok(Some((header.slot(), header.epoch())))
}

/// Get the highest L2 block slot from the database.
///
/// This gets the slot of the highest slot block in the database.
pub(crate) fn get_chain_tip_slot(db: &impl DatabaseBackend) -> Result<u64, DisplayedError> {
    let block_id = get_chain_tip_block_id(db)?;

    // Get block data once and extract slot directly (more efficient than full slot_and_epoch)
    let Some(block_data) = get_l2_block_data(db, block_id)? else {
        return Err(DisplayedError::InternalError(
            "L2 block data not found in database".to_string(),
            Box::new(block_id),
        ));
    };

    Ok(block_data.block().header().slot())
}

/// Get L2 block data by block ID.
pub(crate) fn get_l2_block_data(
    db: &impl DatabaseBackend,
    block_id: L2BlockId,
) -> Result<Option<L2BlockBundle>, DisplayedError> {
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    db.l2_db()
        .get_block_data(block_id)
        .internal_error("Failed to read block data")
}

/// Get L2 block by block ID.
pub(crate) fn get_l2_block(
    db: &impl DatabaseBackend,
    args: GetL2BlockArgs,
) -> Result<(), DisplayedError> {
    // Parse block ID using utility function
    let block_id = parse_l2_block_id(&args.block_id)?;

    // Fetch block status and data
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
    db: &impl DatabaseBackend,
    args: GetL2SummaryArgs,
) -> Result<(), DisplayedError> {
    // Get the tip block (highest slot)
    let tip_block_id = get_chain_tip_block_id(db)?;
    let tip_block_data = get_l2_block_data(db, tip_block_id)?.ok_or_else(|| {
        DisplayedError::InternalError(
            "L2 block data not found in database".to_string(),
            Box::new(tip_block_id),
        )
    })?;
    let tip_slot = tip_block_data.block().header().slot();

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
        #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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

    // Get last epoch from checkpoint database
    let last_epoch = get_last_epoch(db)?;

    // Create the output data structure
    let summary_info = L2SummaryInfo {
        tip_slot,
        tip_block_id: &tip_block_id,
        earliest_slot,
        earliest_block_id: &earliest_block_id,
        last_epoch,
        expected_block_count,
        all_blocks_present,
        missing_slots,
    };

    // Use the output utility
    output(&summary_info, args.output_format)
}
