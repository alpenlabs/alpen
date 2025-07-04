use argh::FromArgs;
use strata_db::traits::{CheckpointDatabase, Database, L1Database};
use strata_primitives::l1::ProtocolOperation;
use tracing::warn;

use crate::{
    cli::OutputFormat,
    cmd::l1::get_l1_horizon_height,
    errors::{DisplayableError, DisplayedError},
};

/// Shows details about an epoch
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-epoch-summary")]
pub(crate) struct GetEpochSummaryArgs {
    /// epoch index; defaults to the latest
    #[argh(positional)]
    pub(crate) epoch_idx: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Shows details about a checkpoint
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-checkpoint-data")]
pub(crate) struct GetCheckpointDataArgs {
    /// checkpoint index; defaults to the latest
    #[argh(positional)]
    pub(crate) checkpoint_idx: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Show a summary of all checkpoints
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-checkpoints-summary")]
pub(crate) struct GetCheckpointsSummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Show details about a specific epoch.
pub(crate) fn get_epoch_summary(
    db: &impl Database,
    args: GetEpochSummaryArgs,
) -> Result<(), DisplayedError> {
    // Determine epoch index
    let epoch_idx = match args.epoch_idx {
        Some(i) => i,
        None => db
            .checkpoint_db()
            .get_last_summarized_epoch()
            .internal_error("Failed to fetch last epoch index")?
            .expect("a valid epoch index"),
    };

    // Fetch epoch summary
    let epoch_commitments = db
        .checkpoint_db()
        .get_epoch_commitments_at(epoch_idx)
        .internal_error("Failed to fetch epoch summary")?;

    if epoch_commitments.is_empty() {
        warn!("no epoch commitments founds");
        return Err(DisplayedError::UserError(
            "Invalid epoch index".to_string(),
            Box::new(epoch_idx),
        ));
    }

    let epoch_summary = db
        .checkpoint_db()
        .get_epoch_summary(*epoch_commitments.first().unwrap())
        .internal_error("Failed to fetch epoch summary")?
        .expect("a valid epoch summary");

    // Print epoch summary
    println!("Epoch idx:  {epoch_idx}");
    println!("Epoch summary: {epoch_summary:#?}");
    Ok(())
}

/// Get details about a specific checkpoint.
pub(crate) fn get_checkpoint_data(
    db: &impl Database,
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

/// Get summary of all checkpoints in the database.
///
/// Also validate that all checkpoints are present in L1 blocks.
pub(crate) fn get_checkpoints_summary(
    db: &impl Database,
    _args: GetCheckpointsSummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();

    let chkpt_db = db.checkpoint_db();
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    println!("Expected total checkpoints: {}", last_idx + 1);
    let mut checkpoint_commitments = Vec::new();
    for idx in 0..=last_idx {
        let entry = chkpt_db
            .get_checkpoint(idx)
            .internal_error(format!("Failed to get checkpoint at index {idx}"))?;

        if let Some(checkpoint_entry) = entry {
            checkpoint_commitments.push(checkpoint_entry.checkpoint.commitment().clone());
        }
    }
    println!(
        "Total checkpoints in checkpoint database: {}",
        checkpoint_commitments.len()
    );

    // Check if all checkpoints are present in L1 blocks
    let (l1_tip_height, _) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    let l1_horizon_height = get_l1_horizon_height(db, l1_tip_height);

    let mut found_checkpoints = 0;
    for l1_height in l1_horizon_height..=l1_tip_height {
        let block_id = l1_db
            .get_canonical_blockid_at_height(l1_height)
            .internal_error("Failed to fetch L1 block id")?
            .expect("a valid block id");
        let manifest = l1_db
            .get_block_manifest(block_id)
            .internal_error("Failed to fetch L1 block id")?
            .expect("a valid manifest");

        manifest
            .txs()
            .iter()
            .flat_map(|tx| tx.protocol_ops())
            .filter_map(|op| match op {
                ProtocolOperation::Checkpoint(signed_checkpoint) => {
                    Some(signed_checkpoint.checkpoint().commitment())
                }
                _ => None,
            })
            .for_each(|commitment| {
                if !checkpoint_commitments.contains(commitment) {
                    println!(
                        "Unexpected checkpoint commitment found in L1 block at height {l1_height}: {commitment:?}"
                    );
                } else {
                    found_checkpoints += 1;
                }
            });
    }

    println!("Checkpoints included in l1 blocks: {found_checkpoints}");

    Ok(())
}
