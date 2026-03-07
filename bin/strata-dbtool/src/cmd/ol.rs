use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{
    BlockStatus, DatabaseBackend, OLBlockDatabase, OLCheckpointDatabase,
};
use strata_identifiers::{Buf32, OLBlockId};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::l1::L1BlockId;

use crate::{
    cli::OutputFormat,
    output::{
        ol::{OLBlockInfo, OLSummaryInfo},
        output,
    },
    utils::block_id::parse_block_id_hex,
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
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get OL block by block ID.
pub(crate) fn get_ol_block(
    db: &impl DatabaseBackend,
    args: GetOLBlockArgs,
) -> Result<(), DisplayedError> {
    let block_id = OLBlockId::from(Buf32::from(parse_block_id_hex(&args.block_id)?));

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
            .map(|manifest| (manifest.height(), manifest.blkid()))
            .collect()
    } else {
        Vec::new()
    };

    // Create the output data structure
    let block_info = OLBlockInfo {
        id: &block_id,
        status: &status,
        header_slot: header.slot(),
        header_epoch: u64::from(header.epoch()),
        header_timestamp: header.timestamp(),
        header_prev_blkid: format!("{:?}", header.parent_blkid()),
        header_body_root: format!("{:?}", header.body_root()),
        header_logs_root: format!("{:?}", header.logs_root()),
        header_state_root: format!("{:?}", header.state_root()),
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

    // Get the earliest block (slot 0) from OL block database.
    let earliest_block_id = get_earliest_ol_block_id(db)?;
    let earliest_block_data = get_ol_block_data(db, earliest_block_id)?.ok_or_else(|| {
        DisplayedError::InternalError(
            "OL block data not found in database".to_string(),
            Box::new(earliest_block_id),
        )
    })?;
    let earliest_slot = earliest_block_data.header().slot();

    // Check for gaps between earliest and tip slots
    let mut missing_slots = Vec::new();
    for slot in earliest_slot..=tip_slot {
        let blocks_at_slot = db
            .ol_block_db()
            .get_blocks_at_height(slot)
            .internal_error(format!("Failed to get blocks at height {slot}"))?;

        if blocks_at_slot.is_empty() {
            missing_slots.push(slot);
        }
    }

    let expected_block_count = tip_slot.saturating_sub(earliest_slot) + 1;
    let all_blocks_present = missing_slots.is_empty();

    // Get last epoch from OL checkpoint database.
    let last_epoch = get_last_ol_checkpoint_epoch(db)?;

    // Create the output data structure
    let summary_info = OLSummaryInfo {
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

fn get_chain_tip_ol_block_id(db: &impl DatabaseBackend) -> Result<OLBlockId, DisplayedError> {
    let tip_slot = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;

    let tip_blocks = db
        .ol_block_db()
        .get_blocks_at_height(tip_slot)
        .internal_error("Failed to fetch OL blocks at tip slot")?;

    tip_blocks.first().copied().ok_or_else(|| {
        DisplayedError::InternalError(
            "No OL blocks found at tip slot".to_string(),
            Box::new(tip_slot),
        )
    })
}

fn get_earliest_ol_block_id(db: &impl DatabaseBackend) -> Result<OLBlockId, DisplayedError> {
    let blocks_at_slot_0 = db
        .ol_block_db()
        .get_blocks_at_height(0)
        .internal_error("Failed to get OL blocks at slot 0")?;

    blocks_at_slot_0.first().copied().ok_or_else(|| {
        DisplayedError::InternalError("No OL blocks found at slot 0".to_string(), Box::new(()))
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

fn get_last_ol_checkpoint_epoch(db: &impl DatabaseBackend) -> Result<Option<u64>, DisplayedError> {
    db.ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last OL checkpoint epoch")
        .map(|v| v.map(u64::from))
}
