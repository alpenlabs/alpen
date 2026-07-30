use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{
    backend::DatabaseBackend, ol_block::OLBlockDatabase, ol_state::OLStateDatabase,
    ol_state_index::OLStateIndexingDatabase,
};
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

use super::{
    checkpoint::{
        delete_ol_checkpoint_data_from_epoch, get_last_ol_checkpoint_epoch,
        get_latest_checkpoint_last_slot, get_latest_finalized_checkpoint_epoch,
    },
    mmr::{
        build_mmr_index_revert_plan, execute_mmr_index_revert_plan, print_mmr_index_revert_summary,
        validate_mmr_index_revert_prefixes,
    },
    ol::{delete_ol_block_data, get_ol_block_slot_and_epoch, mark_ol_block_unchecked},
};
use crate::{
    cli::OutputFormat,
    output::{ol_state::OLStateInfo, output},
    utils::block_id::parse_ol_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-ol-state")]
/// Get OL state at specified block
pub(crate) struct GetOLStateArgs {
    /// OL block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,

    /// L1 reorg-safe depth used to derive finalized checkpoint epoch
    #[argh(option)]
    pub(crate) l1_reorg_safe_depth: u32,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "revert-ol-state")]
/// Revert OL state to specified block
pub(crate) struct RevertOLStateArgs {
    /// target OL block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// delete blocks after target block
    #[argh(switch, short = 'd')]
    pub(crate) delete_blocks: bool,

    /// allow reverting blocks inside checkpointed epoch
    #[argh(switch, short = 'c')]
    pub(crate) revert_checkpointed_blocks: bool,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

    /// L1 reorg-safe depth used to derive finalized checkpoint epoch for safety checks
    #[argh(option)]
    pub(crate) l1_reorg_safe_depth: u32,
}

/// Get OL state at specified block.
pub(crate) fn get_ol_state(
    db: &impl DatabaseBackend,
    args: GetOLStateArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_ol_block_id(&args.block_id)?;
    let (block_slot, block_epoch) =
        get_ol_block_slot_and_epoch(db, block_id)?.ok_or_else(|| {
            DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
        })?;

    let commitment = OLBlockCommitment::new(block_slot, block_id);
    let top_level_state = db
        .ol_state_db()
        .get_toplevel_ol_state(commitment)
        .internal_error("Failed to get OL state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL state not found for block".to_string(),
                Box::new(commitment),
            )
        })?;

    let ol_block = db
        .ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")?
        .ok_or_else(|| {
            DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
        })?;

    // OL state currently exposes ASM-recorded epoch for previous-epoch view.
    let recorded_epoch = top_level_state.epoch_state().asm_recorded_epoch();
    // Finalized epoch should come from client-state declared final epoch (L1-confirmed).
    let finalized_epoch = get_latest_finalized_checkpoint_epoch(db, args.l1_reorg_safe_depth)?
        .unwrap_or_else(EpochCommitment::null);
    let l1_safe_block_height = top_level_state.epoch_state().last_l1_height();
    let ol_state_info = OLStateInfo {
        block_id: &block_id,
        current_slot: block_slot,
        current_epoch: block_epoch,
        is_epoch_finishing: ol_block.header().is_terminal(),
        previous_epoch: recorded_epoch,
        finalized_epoch: &finalized_epoch,
        l1_next_expected_height: l1_safe_block_height.saturating_add(1),
        l1_safe_block_height,
        l1_safe_block_blkid: top_level_state.epoch_state().last_l1_blkid(),
    };

    output(&ol_state_info, args.output_format)
}

/// Revert OL state to specified block.
pub(crate) fn revert_ol_state(
    db: &impl DatabaseBackend,
    args: RevertOLStateArgs,
) -> Result<(), DisplayedError> {
    let target_block_id = parse_ol_block_id(&args.block_id)?;
    let (target_slot, target_epoch) = get_ol_block_slot_and_epoch(db, target_block_id)?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;
    let target_commitment = OLBlockCommitment::new(target_slot, target_block_id);

    let target_block = db
        .ol_block_db()
        .get_block_data(target_block_id)
        .internal_error("Failed to read target OL block")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;
    let target_slot_is_terminal = target_block.header().is_terminal();

    let dry_run = !args.force;

    let chain_tip_slot = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;

    // No-op: target is already at/after current tip.
    if target_slot >= chain_tip_slot {
        println!("No changes would be made.");
        println!(
            "Target slot ({target_slot}) is at or after the chain tip slot ({chain_tip_slot})."
        );
        return Ok(());
    }
    let finalized_epoch = get_latest_finalized_checkpoint_epoch(db, args.l1_reorg_safe_depth)?
        .unwrap_or_else(EpochCommitment::null);
    let finalized_slot = finalized_epoch.last_slot();
    if target_slot < finalized_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    let checkpoint_last_slot = get_latest_checkpoint_last_slot(db)?;
    if !args.revert_checkpointed_blocks && target_slot < checkpoint_last_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside checkpointed epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    let target_state = db
        .ol_state_db()
        .get_toplevel_ol_state(target_commitment)
        .internal_error("Failed to read target OL state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL state not found for target block".to_string(),
                Box::new(target_commitment),
            )
        })?;
    let mmr_revert_plan = build_mmr_index_revert_plan(db, &target_state)?;
    validate_mmr_index_revert_prefixes(db, &mmr_revert_plan)?;

    println!("OL state chain tip slot {chain_tip_slot}");
    println!("OL state finalized slot {finalized_slot}");
    println!("Latest checkpointed slot {checkpoint_last_slot}");
    println!("Revert OL state target slot {target_slot}");
    println!("Target slot is epoch finishing: {target_slot_is_terminal}");
    println!();

    let mut commitments_to_delete = Vec::new();
    let mut blocks_to_mark_unchecked = Vec::new();
    let mut blocks_to_delete = Vec::new();
    let high_watermark_to_revert = db
        .ol_block_db()
        .get_block_high_watermark()
        .internal_error("Failed to get OL block high-watermark")?
        .filter(|high_watermark| high_watermark.slot() > target_slot);

    for slot in target_slot + 1..=chain_tip_slot {
        let block_ids = db
            .ol_block_db()
            .get_blocks_at_height(slot)
            .internal_error(format!("Failed to get OL blocks at slot {slot}"))?;

        for block_id in block_ids {
            let commitment = OLBlockCommitment::new(slot, block_id);
            let has_state = db
                .ol_state_db()
                .get_toplevel_ol_state(commitment)
                .internal_error("Failed to check OL state existence")?
                .is_some();

            if has_state {
                commitments_to_delete.push(commitment);
                if args.delete_blocks {
                    blocks_to_delete.push(block_id);
                } else {
                    blocks_to_mark_unchecked.push(block_id);
                }
            }
        }
    }

    // OL cleanup stores and the MMR index DB do not share a transaction.
    // The MMR preflight above validates counts and peaks before any mutation,
    // but a later storage failure can still leave partial effects.
    if !dry_run {
        for commitment in &commitments_to_delete {
            delete_ol_state_data(db, *commitment)?;
            if args.delete_blocks {
                delete_ol_block_data(db, *commitment.blkid())?;
            } else {
                mark_ol_block_unchecked(db, *commitment.blkid())?;
            }
        }
    }

    let first_epoch_to_clean = if target_slot_is_terminal {
        target_epoch + 1
    } else {
        target_epoch
    };

    if !dry_run {
        revert_indexing(db, target_epoch, target_commitment)?;
        if high_watermark_to_revert.is_some() {
            db.ol_block_db()
                .rollback_block_high_watermark(target_commitment)
                .internal_error("Failed to revert OL block high-watermark")?;
        }
        db.ol_block_db()
            .replace_canonical_suffix_from(target_slot, vec![target_block_id])
            .internal_error("Failed to rewrite canonical OL block index")?;
    }

    let checkpoints_to_delete: Vec<_> = get_last_ol_checkpoint_epoch(db)?
        .map(|last_epoch| (first_epoch_to_clean..=last_epoch).collect())
        .unwrap_or_default();
    let epoch_summaries_to_delete = checkpoints_to_delete.clone();

    if !dry_run {
        delete_ol_checkpoint_data_from_epoch(db, first_epoch_to_clean)?;
    }

    if !dry_run {
        execute_mmr_index_revert_plan(db, &mmr_revert_plan)?;
    }

    let mode = if dry_run { "DRY RUN" } else { "EXECUTED" };
    println!("========================================");
    println!("{mode} SUMMARY");
    println!("========================================");
    println!(
        "OL states/write batches to delete: {}",
        commitments_to_delete.len()
    );
    println!("OL state-indexing revert target: epoch {target_epoch} slot {target_slot}");
    println!(
        "Blocks to mark unchecked: {}",
        blocks_to_mark_unchecked.len()
    );
    println!("Blocks to delete: {}", blocks_to_delete.len());
    if let Some(high_watermark) = high_watermark_to_revert {
        println!(
            "Block high-watermark current: {}",
            format_ol_commitment(&high_watermark)
        );
        println!(
            "Block high-watermark revert target: {}",
            format_ol_commitment(&target_commitment)
        );
    }
    println!("Canonical block index rewrite from slot: {target_slot}");
    println!("Checkpoints to delete: {}", checkpoints_to_delete.len());
    println!(
        "Epoch summaries to delete: {}",
        epoch_summaries_to_delete.len()
    );
    print_mmr_index_revert_summary(&mmr_revert_plan);

    if dry_run {
        println!();
        println!("Use --force to execute these changes.");
    }

    Ok(())
}

/// Formats an OL block commitment with the full block ID for operator summaries.
///
/// [`OLBlockCommitment`] implements [`std::fmt::Display`] with an abbreviated
/// block ID, which is useful in compact logs but not enough for dbtool revert
/// output. The revert summary prints full IDs so operators can copy/paste the
/// exact source and target commitments.
fn format_ol_commitment(commitment: &OLBlockCommitment) -> String {
    format!(
        "{}@{}",
        commitment.slot(),
        hex::encode(commitment.blkid().as_ref())
    )
}

/// Deletes the OL write batch and toplevel state rows keyed by one block commitment.
fn delete_ol_state_data(
    db: &impl DatabaseBackend,
    commitment: OLBlockCommitment,
) -> Result<(), DisplayedError> {
    db.ol_state_db()
        .del_ol_write_batch(commitment)
        .internal_error("Failed to delete OL write batch")?;
    db.ol_state_db()
        .del_toplevel_ol_state(commitment)
        .internal_error("Failed to delete OL state")?;
    Ok(())
}

/// Reverts the indexing DB so its `last_applied_block` and per-block
/// records line up with the reverted tip. Without this, re-execution past
/// `target_slot` errors with `BlockIndexingConflict`.
fn revert_indexing(
    db: &impl DatabaseBackend,
    target_epoch: u32,
    target_commitment: OLBlockCommitment,
) -> Result<(), DisplayedError> {
    db.ol_state_indexing_db()
        .rollback_to_epoch(target_epoch)
        .internal_error("Failed to revert indexing for later epochs")?;
    db.ol_state_indexing_db()
        .rollback_to_block(target_epoch, target_commitment)
        .internal_error("Failed to revert indexing within target epoch")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{
        mmr_index::MmrIndexDatabase,
        ol_block::{BlockStatus, OLBlockDatabase},
        ol_state::OLStateDatabase,
        MmrId,
    };
    use strata_identifiers::{Buf32, Buf64, Hash, OLBlockId};
    use strata_ol_chain_types::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };
    use strata_ol_params::OLParams;
    use strata_ol_state_types::{OLState, MMR_SENTINEL_DUMMY_LEAF_HASH};
    use strata_storage::MmrIndexManager;
    use tokio::runtime::Runtime;

    use super::*;

    fn genesis_target_state() -> OLState {
        OLState::from_genesis_params(&OLParams::default()).expect("valid genesis params")
    }

    fn make_block(slot: u64, epoch: u32, parent_blkid: OLBlockId) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeader::new(
            0,
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

    #[test]
    fn revert_ol_state_force_noops_when_target_is_already_tip() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let target_block = make_block(0, 0, OLBlockId::from(Buf32::zero()));
        let target_block_id = target_block.header().compute_blkid();
        let target_commitment = OLBlockCommitment::new(0, target_block_id);

        db.ol_block_db()
            .put_block_data(target_block)
            .expect("seed target block");
        db.ol_block_db()
            .replace_canonical_suffix_from(0, vec![target_block_id])
            .expect("seed canonical target block");
        db.ol_block_db()
            .set_block_status(target_block_id, BlockStatus::Valid)
            .expect("mark target valid");
        db.ol_state_db()
            .put_toplevel_ol_state(target_commitment, target_state.clone())
            .expect("seed target state");

        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        l1_handle
            .append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)
            .expect("append L1 sentinel");
        l1_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append extra L1 leaf");

        revert_ol_state(
            db.as_ref(),
            RevertOLStateArgs {
                block_id: hex::encode(target_block_id.as_ref()),
                delete_blocks: false,
                revert_checkpointed_blocks: false,
                force: true,
                l1_reorg_safe_depth: 0,
            },
        )
        .expect("revert OL state");

        assert_eq!(
            l1_handle.get_leaf_count_blocking().expect("L1 leaf count"),
            2
        );
        assert_eq!(
            db.ol_block_db()
                .get_block_status(target_block_id)
                .expect("target block status"),
            Some(BlockStatus::Valid)
        );
        assert!(db
            .ol_state_db()
            .get_toplevel_ol_state(target_commitment)
            .expect("target state")
            .is_some());
        assert_eq!(
            db.mmr_index_db()
                .get_leaf_count(MmrId::L1BlockRefs.to_bytes())
                .expect("L1 leaf count row"),
            2
        );
    }

    #[test]
    fn revert_ol_state_rejects_non_prefix_mmr() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let tip_state = genesis_target_state();
        let target_block = make_block(0, 0, OLBlockId::from(Buf32::zero()));
        let target_block_id = target_block.header().compute_blkid();
        let target_commitment = OLBlockCommitment::new(0, target_block_id);
        let tip_block = make_block(1, 0, target_block_id);
        let tip_block_id = tip_block.header().compute_blkid();
        let tip_commitment = OLBlockCommitment::new(1, tip_block_id);

        db.ol_block_db()
            .put_block_data(target_block)
            .expect("seed target block");
        db.ol_block_db()
            .put_block_data(tip_block)
            .expect("seed tip block");
        db.ol_block_db()
            .replace_canonical_suffix_from(0, vec![target_block_id, tip_block_id])
            .expect("seed canonical blocks");
        db.ol_block_db()
            .set_block_status(target_block_id, BlockStatus::Valid)
            .expect("mark target valid");
        db.ol_block_db()
            .set_block_status(tip_block_id, BlockStatus::Valid)
            .expect("mark tip valid");
        db.ol_state_db()
            .put_toplevel_ol_state(target_commitment, target_state)
            .expect("seed target state");
        db.ol_state_db()
            .put_toplevel_ol_state(tip_commitment, tip_state)
            .expect("seed tip state");

        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        l1_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append non-target L1 first leaf");
        l1_handle
            .append_leaf_blocking(Hash::from([0x22; 32]))
            .expect("append extra L1 leaf");

        let err = revert_ol_state(
            db.as_ref(),
            RevertOLStateArgs {
                block_id: hex::encode(target_block_id.as_ref()),
                delete_blocks: false,
                revert_checkpointed_blocks: false,
                force: true,
                l1_reorg_safe_depth: 0,
            },
        )
        .expect_err("non-prefix MMR should abort revert");

        assert!(err.to_string().contains("does not match target prefix"));
        assert!(db
            .ol_state_db()
            .get_toplevel_ol_state(tip_commitment)
            .expect("tip state")
            .is_some());
        assert_eq!(
            db.ol_block_db()
                .get_block_status(tip_block_id)
                .expect("tip block status"),
            Some(BlockStatus::Valid)
        );
        assert_eq!(
            l1_handle.get_leaf_count_blocking().expect("L1 leaf count"),
            2
        );
    }
}
