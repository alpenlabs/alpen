use argh::FromArgs;
use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, DatabaseBackend, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_state::{block::L2BlockBundle, header::L2Header};

use super::checkpoint::get_last_epoch;
use crate::cli::OutputFormat;

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

/// Get the latest L2 block from the database.
///
/// This finds the highest slot block in the database.
pub(crate) fn get_latest_l2_block_id(
    db: &impl DatabaseBackend,
) -> Result<L2BlockId, DisplayedError> {
    db.l2_db()
        .get_tip_block()
        .internal_error("Failed to get latest L2 block")
}

/// Get the earliest L2 block id from the database.
pub(crate) fn get_earliest_l2_block_id(
    db: &impl DatabaseBackend,
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

/// Get L2 block data by block ID.
pub(crate) fn get_l2_block_data(
    db: &impl DatabaseBackend,
    block_id: L2BlockId,
) -> Result<Option<L2BlockBundle>, DisplayedError> {
    db.l2_db()
        .get_block_data(block_id)
        .internal_error("Failed to read block data")
}

/// Get L2 block by block ID.
pub(crate) fn get_l2_block(
    db: &impl DatabaseBackend,
    args: GetL2BlockArgs,
) -> Result<(), DisplayedError> {
    // Convert String to L2BlockId
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let block_id = L2BlockId::from(Buf32::from(bytes));

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

    // Print in porcelain format
    println!("l2_block.id {block_id:?}");
    println!("l2_block.status {status:?}");
    println!("l2_block.header.slot {}", header.slot());
    println!("l2_block.header.parent_blkid {:?}", header.parent());
    println!("l2_block.header.state_root {:?}", header.state_root());
    println!(
        "l2_block.header.l1_payload_hash {:?}",
        header.l1_payload_hash()
    );
    println!(
        "l2_block.header.exec_payload_hash {:?}",
        header.exec_payload_hash()
    );
    println!("l2_block.header.epoch {}", header.epoch());
    println!("l2_block.header.timestamp {}", header.timestamp());

    // Print L1 segment information
    let l1_segment = l2_block.body().l1_segment();
    println!(
        "l2_block.l1_segment.new_manifests_count {}",
        l1_segment.new_manifests().len()
    );

    for (index, manifest) in l1_segment.new_manifests().iter().enumerate() {
        println!(
            "l2_block.l1_segment.manifest_{index}.height {}",
            manifest.height()
        );
        println!(
            "l2_block.l1_segment.manifest_{index}.blkid {:?}",
            manifest.blkid()
        );
    }

    Ok(())
}

/// Get L2 summary - check all L2 blocks exist in database.
pub(crate) fn get_l2_summary(
    db: &impl DatabaseBackend,
    _args: GetL2SummaryArgs,
) -> Result<(), DisplayedError> {
    // Get the tip block (highest slot)
    let tip_block_id = get_latest_l2_block_id(db)?;
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

    // Print in porcelain format
    println!("l2_summary.tip_slot {tip_slot}");
    println!("l2_summary.tip_block_id {tip_block_id:?}");
    println!("l2_summary.earliest_slot {earliest_slot}");
    println!("l2_summary.earliest_block_id {earliest_block_id:?}");
    println!(
        "l2_summary.last_epoch {}",
        last_epoch.map_or("None".to_string(), |e| e.to_string())
    );
    println!("l2_summary.expected_block_count {expected_block_count}");
    println!("l2_summary.all_blocks_present {all_blocks_present}");

    if !missing_slots.is_empty() {
        println!("l2_summary.missing_slots_count {}", missing_slots.len());
        for slot in missing_slots {
            println!("l2_summary.missing_slot {slot}");
        }
    }

    Ok(())
}
