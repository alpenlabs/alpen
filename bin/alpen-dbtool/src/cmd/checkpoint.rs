use std::sync::Arc;

use clap::Args;
use strata_db::traits::{CheckpointDatabase, Database, L1Database};
use strata_primitives::l1::ProtocolOperation;
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to show details about a checkpoint.
#[derive(Args, Debug)]
pub struct GetCheckpointDataArgs {
    /// Checkpoint index; defaults to the latest
    checkpoint_idx: Option<u64>,
}

/// Arguments to show a summary of all checkpoints.
#[derive(Args, Debug)]
pub struct GetCheckpointsSummaryArgs {}

pub fn get_checkpoint_data(
    db: Arc<CommonDb>,
    args: GetCheckpointDataArgs,
) -> Result<(), DisplayedError> {
    // Determine checkpoint index
    let checkpoint_idx = match args.checkpoint_idx {
        Some(i) => i,
        None => db
            .checkpoint_db()
            .get_last_checkpoint_idx()
            .internal_error("Failed to fetch last checkpoint index")?
            .expect("a valid checkpoint index"),
    };

    // Fetch checkpoint data
    let entry = db
        .checkpoint_db()
        .get_checkpoint(checkpoint_idx)
        .internal_error("Failed to fetch checkpoint data")?
        .expect("a valid checkpoint entry");

    let checkpoint_commitment = entry.checkpoint.commitment();

    // Print checkpoint information
    println!("Checkpoint idx:  {checkpoint_idx}");
    println!("Checkpoint commitment: {checkpoint_commitment:#?}");
    println!("Checkpoint status: {:?}", entry.confirmation_status);
    println!("Proving status: {:?}", entry.proving_status);

    Ok(())
}

pub fn get_checkpoints_summary(
    db: Arc<CommonDb>,
    _args: GetCheckpointsSummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();

    let chkpt_db = db.checkpoint_db();
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    println!("Expected total checkpoints: {}", last_idx + 1);
    let mut checkpoints_count = 0;
    for idx in 0..=last_idx {
        let entry = chkpt_db
            .get_checkpoint(idx)
            .internal_error(format!("Failed to get checkpoint at index {idx}"))?;

        if entry.is_some() {
            checkpoints_count += 1;
        }
    }
    println!(
        "Total checkpoints in checkpoint database: {}",
        checkpoints_count
    );

    // Check all checkpoints are in L1 blocks.
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

    // Check checkpoints in blocks from apparent genesis to tip
    let total_checkpoint_txs: usize = (apparent_genesis_l1_height..=l1_tip_height)
        .filter_map(|l1_height| {
            let block_id = l1_db
                .get_canonical_blockid_at_height(l1_height)
                .ok()
                .flatten()?;

            let manifest = l1_db.get_block_manifest(block_id).ok().flatten()?;

            let count = manifest
                .txs()
                .iter()
                .filter(|tx| {
                    tx.protocol_ops()
                        .iter()
                        .any(|op| matches!(op, ProtocolOperation::Checkpoint(_)))
                })
                .count();

            Some(count)
        })
        .sum();

    println!(
        "Total checkpoint transactions found in L1 blocks from apparent genesis to tip: {}",
        total_checkpoint_txs
    );

    Ok(())
}
