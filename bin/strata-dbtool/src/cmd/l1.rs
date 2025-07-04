use argh::FromArgs;
use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{Database, L1Database};
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockId, ProtocolOperation},
};
use tracing::warn;

use crate::{cli::OutputFormat, cmd::client_state::get_latest_client_state_update};

/// Shows details about an L1 manifest
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-l1-manifest")]
pub(crate) struct GetL1ManifestArgs {
    /// block height; defaults to the chain tip
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Shows a summary of all L1 manifests
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-l1-summary")]
pub(crate) struct GetL1SummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get details about a specific L1 block manifest.
pub(crate) fn get_l1_manifest(
    db: &impl Database,
    args: GetL1ManifestArgs,
) -> Result<(), DisplayedError> {
    // Convert String to L1BlockId
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let block_id = L1BlockId::from(Buf32::from(bytes));

    // Get block manifest
    let l1_block_manifest = db
        .l1_db()
        .get_block_manifest(block_id)
        .internal_error(format!(
            "Failed to get block manifest for id {}",
            args.block_id
        ))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No block manifest found for given block id".into(),
                Box::new(args.block_id),
            )
        })?;

    // Basic block info
    println!(
        "L1 block height: {}, id: {block_id:?}",
        l1_block_manifest.height()
    );

    // Number of transactions
    println!(
        "L1 block has {} transaction(s)",
        l1_block_manifest.txs().len()
    );

    println!(
        "L1 block epoch (this looks wrong): {:?}",
        l1_block_manifest.epoch()
    );

    // Print relevant transactions
    for tx in l1_block_manifest.txs().iter() {
        for proto_op in tx.protocol_ops().iter() {
            match proto_op {
                ProtocolOperation::Checkpoint(signed_checkpoint) => {
                    println!(
                        "checkpoint commitment: {:?}",
                        signed_checkpoint.checkpoint().commitment()
                    );
                }
                ProtocolOperation::DaCommitment(da_commitment) => {
                    println!("DA commitment: {da_commitment:?}");
                }
                ProtocolOperation::WithdrawalFulfillment(wf_info) => {
                    println!("checkpoint commitment: {wf_info:?}");
                }
                _ => continue,
            }
        }
    }

    Ok(())
}

/// Get summary of L1 manifests in the database.
pub(crate) fn get_l1_summary(
    db: &impl Database,
    _args: GetL1SummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();
    let (l1_tip_height, l1_tip_block_id) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    println!("L1 tip height: {l1_tip_height}, block id {l1_tip_block_id:?}");

    let l1_horizon_height = get_l1_horizon_height(db, l1_tip_height);
    if l1_horizon_height == l1_tip_height {
        warn!("Missing all l1 blocks from horizon to tip.");
        return Ok(());
    }

    let horizon_l1_block_id = l1_db
        .get_canonical_blockid_at_height(l1_horizon_height)
        .internal_error("Failed to read L1 genesis block id")?
        .expect("valid genesis block id");

    let (latest_client_state, latest_update_idx) = get_latest_client_state_update(db, None)?;
    let genesis_l1_height = latest_client_state.state().genesis_l1_height();

    println!("L1 horizon height: {l1_horizon_height}, block id {horizon_l1_block_id:?}");
    println!(
        "Genesis l1 height: {genesis_l1_height:?}, expected number of l1 blocks (horizon height to tip) {latest_update_idx},
        number of client state updates: {}",
        l1_tip_height.saturating_sub(l1_horizon_height) + 1,
    );

    // Check if all L1 blocks from L1 horizon to tip are present
    let all_l1_manifests_present = (l1_horizon_height..=l1_tip_height).all(|l1_height| {
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

    if all_l1_manifests_present {
        println!("All expected l1 block manifests found in L1Database.")
    }

    Ok(())
}

/// Get the L1 horizon height, i.e., the height of the first L1 block in the database
pub(super) fn get_l1_horizon_height(db: &impl Database, l1_tip_height: u64) -> u64 {
    let l1_db = db.l1_db();

    (0..=l1_tip_height)
        .find(|&height| matches!(l1_db.get_canonical_blockid_at_height(height), Ok(Some(_))))
        .unwrap_or(l1_tip_height)
}
