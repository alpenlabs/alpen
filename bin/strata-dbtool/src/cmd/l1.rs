use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{DatabaseBackend, L1Database};
use strata_primitives::l1::{L1BlockId, ProtocolOperation};
use tracing::warn;

use crate::{
    cli::OutputFormat, cmd::client_state::get_latest_client_state_update,
    utils::block_id::parse_l1_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-manifest")]
/// Get L1 manifest
pub(crate) struct GetL1ManifestArgs {
    /// block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-summary")]
/// Get L1 summary
pub(crate) struct GetL1SummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get the L1 chain tip (height, block_id) of the canonical chain tip.
pub(crate) fn get_l1_chain_tip(
    db: &impl DatabaseBackend,
) -> Result<(u64, L1BlockId), DisplayedError> {
    db.l1_db()
        .get_canonical_chain_tip()
        .internal_error("Failed to get L1 tip")?
        .ok_or_else(|| {
            DisplayedError::InternalError("L1 tip not found in database".to_string(), Box::new(()))
        })
}

/// Get L1 block ID at a specific height.
pub(crate) fn get_l1_block_id_at_height(
    db: &impl DatabaseBackend,
    height: u64,
) -> Result<L1BlockId, DisplayedError> {
    db.l1_db()
        .get_canonical_blockid_at_height(height)
        .internal_error(format!("Failed to get L1 block ID at height {height}"))?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "L1 block id not found for height".to_string(),
                Box::new(height),
            )
        })
}

/// Get L1 block manifest by block ID.
pub(crate) fn get_l1_block_manifest(
    db: &impl DatabaseBackend,
    block_id: L1BlockId,
) -> Result<Option<strata_primitives::l1::L1BlockManifest>, DisplayedError> {
    db.l1_db()
        .get_block_manifest(block_id)
        .internal_error(format!("Failed to get block manifest for id {block_id:?}",))
}

/// Get L1 manifest by block ID.
pub(crate) fn get_l1_manifest(
    db: &impl DatabaseBackend,
    args: GetL1ManifestArgs,
) -> Result<(), DisplayedError> {
    // Parse block ID using utility function
    let block_id = parse_l1_block_id(&args.block_id)?;

    // Get block manifest using helper function
    let l1_block_manifest = get_l1_block_manifest(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No L1 block manifest found for block id".to_string(),
            Box::new(block_id),
        )
    })?;

    // Print in porcelain format
    println!("l1_block.height {}", l1_block_manifest.height());
    println!("l1_block.blkid {block_id:?}");
    println!("l1_block.tx_count {}", l1_block_manifest.txs().len());

    // Print relevant transactions
    for (index, tx) in l1_block_manifest.txs().iter().enumerate() {
        for proto_op in tx.protocol_ops().iter() {
            match proto_op {
                ProtocolOperation::Checkpoint(signed_checkpoint) => {
                    println!(
                        "tx_{index}.checkpoint.signature {:?}",
                        signed_checkpoint.signature()
                    );
                    let batch_info = signed_checkpoint.checkpoint().batch_info();
                    println!("tx_{index}.checkpoint.batch.epoch {}", batch_info.epoch());
                    println!(
                        "tx_{index}.checkpoint.batch.l1_range.start.height {}",
                        batch_info.l1_range.0.height()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l1_range.start.blkid {:?}",
                        batch_info.l1_range.0.blkid()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l1_range.end.height {}",
                        batch_info.l1_range.1.height()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l1_range.end.blkid {:?}",
                        batch_info.l1_range.1.blkid()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l2_range.start.slot {}",
                        batch_info.l2_range.0.slot()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l2_range.start.blkid {:?}",
                        batch_info.l2_range.0.blkid()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l2_range.end.slot {}",
                        batch_info.l2_range.1.slot()
                    );
                    println!(
                        "tx_{index}.checkpoint.batch.l2_range.end.blkid {:?}",
                        batch_info.l2_range.1.blkid()
                    );

                    let batch_transition = signed_checkpoint.checkpoint().batch_transition();
                    println!(
                        "tx_{index}.checkpoint.batch_transition.chainstate.pre_root {:?}",
                        batch_transition.chainstate_transition.pre_state_root
                    );
                    println!(
                        "tx_{index}.checkpoint.batch_transition.chainstate.post_root {:?}",
                        batch_transition.chainstate_transition.post_state_root
                    );
                    println!(
                        "tx_{index}.checkpoint.batch_transition.tx_filter.pre_config_hash {:?}",
                        batch_transition.tx_filters_transition.pre_config_hash
                    );
                    println!(
                        "tx_{index}.checkpoint.batch_transition.tx_filter.post_config_hash {:?}",
                        batch_transition.tx_filters_transition.post_config_hash
                    );
                }
                ProtocolOperation::DaCommitment(da_commitment) => {
                    println!("DA commitment: {da_commitment:?}");
                }
                ProtocolOperation::WithdrawalFulfillment(wf_info) => {
                    println!("Withdrawal fulfillment: {wf_info:?}");
                }
                _ => continue,
            }
        }
    }

    Ok(())
}

/// Get L1 summary - check all L1 block manifests exist in database.
pub(crate) fn get_l1_summary(
    db: &impl DatabaseBackend,
    _args: GetL1SummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();

    // Use helper function to get L1 tip
    let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

    let (client_state_update, _) = get_latest_client_state_update(db, None)?;
    let (client_state, _) = client_state_update.into_parts();
    let horizon_l1_height = client_state.horizon_l1_height();
    let genesis_l1_height = client_state.genesis_l1_height();

    if horizon_l1_height == l1_tip_height {
        warn!("Missing all l1 blocks from horizon to tip.");
        return Ok(());
    }

    // Use helper function to get horizon block ID
    let horizon_l1_blkid = get_l1_block_id_at_height(db, horizon_l1_height)?;

    println!("l1_summary.tip_height {l1_tip_height}");
    println!("l1_summary.tip_blkid {l1_tip_block_id:?}");
    println!("l1_summary.horizon_height {horizon_l1_height}");
    println!("l1_summary.horizon_blkid {horizon_l1_blkid:?}");
    println!("l1_summary.genesis_height: {genesis_l1_height:?}");
    println!(
        "l1_summary.expected_l1_block_count {}",
        l1_tip_height.saturating_sub(horizon_l1_height) + 1
    );

    // Check if all L1 blocks from L1 horizon to tip are present
    let all_l1_manifests_present = (horizon_l1_height..=l1_tip_height).all(|l1_height| {
        let Some(block_id) = l1_db
            .get_canonical_blockid_at_height(l1_height)
            .ok()
            .flatten()
        else {
            println!("Missing block id at height {l1_height}");
            return false;
        };

        if l1_db.get_block_manifest(block_id).ok().flatten().is_none() {
            println!("Missing manifest at height {l1_height}: block id {block_id:?}",);
            return false;
        }

        true
    });

    println!("l1_summary.all_manifests_in_l1_db {all_l1_manifests_present}");

    Ok(())
}
