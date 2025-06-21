use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{Database, L1Database};
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockId, ProtocolOperation},
};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to show details about an L1 manifest.
#[derive(Args, Debug)]
pub struct GetL1ManifestArgs {
    /// Block height; defaults to the chain tip
    #[arg(value_name = "L1_BLOCK_ID")]
    pub block_id: String,
}

/// Arguments to show a summary of all L1 manifests.
#[derive(Args, Debug)]
pub struct GetL1SummaryArgs {}

/// Get details about a specific L1 block manifest.
pub fn get_l1_manifest(db: Arc<CommonDb>, args: GetL1ManifestArgs) -> Result<(), DisplayedError> {
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

    println!("L1 block : {:?}", l1_block_manifest.epoch());

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
                    println!("DA commitment: {:?}", da_commitment);
                }
                ProtocolOperation::WithdrawalFulfillment(wf_info) => {
                    println!("checkpoint commitment: {:?}", wf_info);
                }
                _ => continue,
            }
        }
    }

    Ok(())
}

/// Get summary of L1 manifests in the database.
pub fn get_l1_summary(db: Arc<CommonDb>, _args: GetL1SummaryArgs) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();
    let (l1_tip_height, l1_tip_block_id) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    println!(
        "L1 tip height: {}, block id {:?}",
        l1_tip_height, l1_tip_block_id
    );

    let apparent_genesis_l1_height = (0..=l1_tip_height)
        .rev()
        .find(
            |&height| match l1_db.get_canonical_blockid_at_height(height) {
                Ok(Some(_)) => false, // keep searching
                _ => true,            // break here, found missing or error
            },
        )
        .map(|h| h + 1) // next known good height
        .unwrap_or(l1_tip_height);

    let genesis_l1_block_id = l1_db
        .get_canonical_blockid_at_height(apparent_genesis_l1_height)
        .internal_error("Failed to read L1 genesis block id")?
        .expect("valid genesis block id");

    println!(
        "Apparent genesis l1 height: {}, block id {:?}",
        apparent_genesis_l1_height, genesis_l1_block_id
    );

    // Check if all L1 blocks from apparent genesis to tip are present
    let all_l1_manifests_present = (apparent_genesis_l1_height..=l1_tip_height).all(|l1_height| {
        let Some(block_id) = l1_db
            .get_canonical_blockid_at_height(l1_height)
            .ok()
            .flatten()
        else {
            println!("Missing block id at height {l1_height}");
            return false;
        };

        if l1_db.get_block_manifest(block_id).ok().flatten().is_none() {
            println!(
                "Missing manifest at height {}: block id {:?}",
                l1_height, block_id
            );
            return false;
        }

        true
    });

    if all_l1_manifests_present {
        println!("All expected l1 block manifests found in L1Database.")
    }

    Ok(())
}
