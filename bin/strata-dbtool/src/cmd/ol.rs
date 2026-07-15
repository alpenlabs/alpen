use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{
    backend::DatabaseBackend,
    ol_block::{BlockStatus, OLBlockDatabase},
    ol_state::OLStateDatabase,
};
use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types::OLBlock;
use strata_primitives::l1::L1BlockId;

use crate::{
    cli::OutputFormat,
    cmd::checkpoint::get_last_ol_checkpoint_epoch,
    output::{
        ol::{OLBlockDeleteInfo, OLBlockInfo, OLBlocksAtSlotInfo, OLSummaryInfo},
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

/// Get all OL block IDs stored for a slot.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-ol-blocks-at-slot")]
pub(crate) struct GetOLBlocksAtSlotArgs {
    /// OL slot
    #[argh(positional)]
    pub(crate) slot: Slot,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Delete one OL block (block data, status, and slot-index entry).
///
/// Intended for pruning an orphaned sibling block left behind by a reorg,
/// e.g. when the slot index still lists the orphan ahead of the canonical
/// block. Refuses to delete a block that is alone at its slot, that has a
/// stored child, or that is the current block high-watermark.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "delete-ol-block")]
pub(crate) struct DeleteOLBlockArgs {
    /// OL block id (hex)
    #[argh(positional)]
    pub(crate) block_id: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

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

    // Create manifest data from the block's ASM manifests (if present).
    let manifest_data: Vec<(u64, &L1BlockId)> = if let Some(manifests) = ol_block.body().manifests()
    {
        manifests
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
    // Get the canonical tip block from OL block database.
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

/// Gets all OL block IDs stored for a slot.
pub(crate) fn get_ol_blocks_at_slot(
    db: &impl DatabaseBackend,
    args: GetOLBlocksAtSlotArgs,
) -> Result<(), DisplayedError> {
    let block_ids = db
        .ol_block_db()
        .get_blocks_at_height(args.slot)
        .internal_error(format!("Failed to get blocks at slot {}", args.slot))?;

    let info = OLBlocksAtSlotInfo {
        slot: args.slot,
        count: block_ids.len(),
        block_ids: &block_ids,
    };

    output(&info, args.output_format)
}

/// Deletes one OL block after checking it is a childless sibling fork.
pub(crate) fn delete_ol_block(
    db: &impl DatabaseBackend,
    args: DeleteOLBlockArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_ol_block_id(&args.block_id)?;

    let block = get_ol_block_data(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
    })?;
    let slot = block.header().slot();

    let blocks_at_slot = db
        .ol_block_db()
        .get_blocks_at_height(slot)
        .internal_error(format!("Failed to get blocks at slot {slot}"))?;

    // Deleting the only block at a slot would punch a hole in the chain.
    let remaining_block_ids: Vec<OLBlockId> = blocks_at_slot
        .iter()
        .copied()
        .filter(|id| *id != block_id)
        .collect();
    if remaining_block_ids.is_empty() {
        return Err(DisplayedError::UserError(
            "Refusing to delete the only block at slot".to_string(),
            Box::new(slot),
        ));
    }

    let canonical_block = db
        .ol_block_db()
        .get_canonical_block(slot)
        .internal_error(format!("Failed to get canonical block at slot {slot}"))?;
    if canonical_block == Some(block_id) {
        return Err(DisplayedError::UserError(
            "Refusing to delete canonical OL block".to_string(),
            Box::new(block_id),
        ));
    }

    // A stored child means this block is (or was) extended by some chain;
    // deleting it would leave that child without a parent in the database.
    let Some(next_slot) = slot.checked_add(1) else {
        return Err(DisplayedError::UserError(
            "Refusing to delete max-slot OL block".to_string(),
            Box::new(block_id),
        ));
    };
    let blocks_at_next_slot = db
        .ol_block_db()
        .get_blocks_at_height(next_slot)
        .internal_error(format!("Failed to get blocks at slot {next_slot}"))?;
    for child_id in blocks_at_next_slot {
        let Some(child) = get_ol_block_data(db, child_id)? else {
            continue;
        };
        if *child.header().parent_blkid() == block_id {
            return Err(DisplayedError::UserError(
                "Refusing to delete block with a stored child".to_string(),
                Box::new(child_id),
            ));
        }
    }

    let block_high_watermark = db
        .ol_block_db()
        .get_block_high_watermark()
        .internal_error("Failed to read OL block high-watermark")?;
    if let Some(high_watermark) = block_high_watermark {
        if *high_watermark.blkid() == block_id {
            return Err(DisplayedError::UserError(
                "Refusing to delete current OL block high-watermark".to_string(),
                Box::new(high_watermark),
            ));
        }
    }

    let commitment = OLBlockCommitment::new(slot, block_id);
    ensure_ol_block_not_applied(db, commitment)?;

    if args.force {
        delete_ol_block_data(db, block_id)?;
    }

    let info = OLBlockDeleteInfo {
        block_id: &block_id,
        slot,
        remaining_block_ids: &remaining_block_ids,
        dry_run: !args.force,
    };
    output(&info, args.output_format)
}

/// Marks one OL block as unchecked while keeping its block data.
pub(crate) fn mark_ol_block_unchecked(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<(), DisplayedError> {
    db.ol_block_db()
        .set_block_status(block_id, BlockStatus::Unchecked)
        .internal_error("Failed to set OL block status")?;

    Ok(())
}

/// Deletes one OL block's data, status, and slot-index entry.
pub(crate) fn delete_ol_block_data(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<(), DisplayedError> {
    db.ol_block_db()
        .del_block_data(block_id)
        .internal_error("Failed to delete OL block")?;

    Ok(())
}

/// Refuses orphan-only deletion when block execution artifacts are present.
///
/// Raw block storage does not append MMR leaves. If write batch or state
/// artifacts exist for this commitment, the block was applied far enough that
/// `revert-ol-state` is the correct recovery path.
fn ensure_ol_block_not_applied(
    db: &impl DatabaseBackend,
    commitment: OLBlockCommitment,
) -> Result<(), DisplayedError> {
    let has_write_batch = db
        .ol_state_db()
        .get_ol_write_batch(commitment)
        .internal_error("Failed to check OL write batch existence")?
        .is_some();
    let has_state = db
        .ol_state_db()
        .get_toplevel_ol_state(commitment)
        .internal_error("Failed to check OL state existence")?
        .is_some();

    if has_write_batch || has_state {
        return Err(DisplayedError::UserError(
            "Refusing to delete applied OL block with OL state".to_string(),
            Box::new(commitment),
        ));
    }

    Ok(())
}

fn get_chain_tip_ol_block_id(db: &impl DatabaseBackend) -> Result<OLBlockId, DisplayedError> {
    Ok(*get_canonical_ol_tip(db)?.blkid())
}

pub(crate) fn get_canonical_ol_tip(
    db: &impl DatabaseBackend,
) -> Result<OLBlockCommitment, DisplayedError> {
    let tip_slot = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;
    let tip_blkid = get_canonical_ol_block_at_slot(db, tip_slot)?;
    Ok(OLBlockCommitment::new(tip_slot, tip_blkid))
}

pub(crate) fn get_canonical_ol_block_at_slot(
    db: &impl DatabaseBackend,
    slot: Slot,
) -> Result<OLBlockId, DisplayedError> {
    db.ol_block_db()
        .get_canonical_block(slot)
        .internal_error("Failed to fetch canonical OL block at slot")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "No canonical OL block found at slot".to_string(),
                Box::new(slot),
            )
        })
}

pub(crate) fn get_ol_block_slot_and_epoch(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<Option<(Slot, Epoch)>, DisplayedError> {
    let Some(block) = db
        .ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")?
    else {
        return Ok(None);
    };

    Ok(Some((block.header().slot(), block.header().epoch())))
}

fn get_ol_block_data(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<Option<OLBlock>, DisplayedError> {
    db.ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::{test_utils::get_test_sled_backend, SledBackend};
    use strata_db_types::{ol_block::OLBlockDatabase, ol_state::OLStateDatabase};
    use strata_identifiers::{Buf32, Buf64, L1BlockCommitment};
    use strata_ol_chain_types::{
        BlockFlags, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };
    use strata_ol_params::OLParams;
    use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};

    use super::*;

    fn make_block(slot: u64, epoch: u32, parent_blkid: OLBlockId) -> OLBlock {
        make_block_with_timestamp(slot, epoch, parent_blkid, 0)
    }

    fn make_block_with_timestamp(
        slot: u64,
        epoch: u32,
        parent_blkid: OLBlockId,
        timestamp: u64,
    ) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeader::new(
            timestamp,
            BlockFlags::zero(),
            slot,
            epoch,
            parent_blkid,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );

        OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
    }

    fn genesis_state() -> OLState {
        OLState::from_genesis_params(&OLParams::new_empty(
            L1BlockCommitment::default(),
            strata_bridge_params::BridgeParams::default(),
        ))
        .expect("valid genesis params")
    }

    fn seed_sibling_blocks() -> (Arc<SledBackend>, OLBlock, OLBlock) {
        let db = get_test_sled_backend();
        let parent_id = OLBlockId::from(Buf32::from([0x01; 32]));
        let block_to_delete = make_block_with_timestamp(1, 0, parent_id, 1);
        let sibling_to_keep = make_block_with_timestamp(1, 0, parent_id, 2);

        db.ol_block_db()
            .put_block_data(block_to_delete.clone())
            .expect("seed block to delete");
        db.ol_block_db()
            .put_block_data(sibling_to_keep.clone())
            .expect("seed sibling block");

        (db, block_to_delete, sibling_to_keep)
    }

    fn delete_args(block_id: OLBlockId) -> DeleteOLBlockArgs {
        DeleteOLBlockArgs {
            block_id: hex::encode(block_id.as_ref()),
            force: true,
            output_format: OutputFormat::Porcelain,
        }
    }

    #[test]
    fn delete_ol_block_force_deletes_unapplied_sibling() {
        let (db, block_to_delete, sibling_to_keep) = seed_sibling_blocks();
        let block_to_delete_id = block_to_delete.header().compute_blkid();
        let sibling_to_keep_id = sibling_to_keep.header().compute_blkid();

        delete_ol_block(db.as_ref(), delete_args(block_to_delete_id)).expect("delete OL block");

        assert!(db
            .ol_block_db()
            .get_block_data(block_to_delete_id)
            .expect("read deleted block")
            .is_none());
        assert!(db
            .ol_block_db()
            .get_block_data(sibling_to_keep_id)
            .expect("read sibling block")
            .is_some());
        assert!(!db
            .ol_block_db()
            .get_blocks_at_height(1)
            .expect("read slot blocks")
            .contains(&block_to_delete_id));
    }

    #[test]
    fn delete_ol_block_rejects_block_with_write_batch() {
        let (db, block_to_delete, _) = seed_sibling_blocks();
        let block_to_delete_id = block_to_delete.header().compute_blkid();
        let commitment = OLBlockCommitment::new(1, block_to_delete_id);
        db.ol_state_db()
            .put_ol_write_batch(commitment, WriteBatch::<OLAccountState>::default())
            .expect("seed write batch");

        let err = delete_ol_block(db.as_ref(), delete_args(block_to_delete_id))
            .expect_err("applied block should be rejected");

        assert!(err
            .to_string()
            .contains("Refusing to delete applied OL block"));
        assert!(db
            .ol_block_db()
            .get_block_data(block_to_delete_id)
            .expect("read rejected block")
            .is_some());
    }

    #[test]
    fn delete_ol_block_rejects_block_with_toplevel_state() {
        let (db, block_to_delete, _) = seed_sibling_blocks();
        let block_to_delete_id = block_to_delete.header().compute_blkid();
        let commitment = OLBlockCommitment::new(1, block_to_delete_id);
        db.ol_state_db()
            .put_toplevel_ol_state(commitment, genesis_state())
            .expect("seed OL state");

        let err = delete_ol_block(db.as_ref(), delete_args(block_to_delete_id))
            .expect_err("applied block should be rejected");

        assert!(err
            .to_string()
            .contains("Refusing to delete applied OL block"));
        assert!(db
            .ol_block_db()
            .get_block_data(block_to_delete_id)
            .expect("read rejected block")
            .is_some());
    }

    #[test]
    fn mark_ol_block_unchecked_keeps_block_data() {
        let db = get_test_sled_backend();
        let block = make_block(1, 0, OLBlockId::from(Buf32::from([0x01; 32])));
        let block_id = block.header().compute_blkid();
        db.ol_block_db().put_block_data(block).expect("seed block");
        db.ol_block_db()
            .set_block_status(block_id, BlockStatus::Valid)
            .expect("mark valid");

        mark_ol_block_unchecked(db.as_ref(), block_id).expect("mark block unchecked");

        assert!(db
            .ol_block_db()
            .get_block_data(block_id)
            .expect("read block")
            .is_some());
        assert_eq!(
            db.ol_block_db()
                .get_block_status(block_id)
                .expect("read block status"),
            Some(BlockStatus::Unchecked)
        );
    }
}
