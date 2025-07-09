use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::{
    traits::{CheckpointDatabase, Database, L1Database},
    types::{CheckpointConfStatus, CheckpointProvingStatus},
};
use strata_primitives::{
    batch::{CheckpointCommitment, EpochSummary},
    l1::ProtocolOperation,
};
use strata_state::client_state::CheckpointL1Ref;
use tracing::warn;

use super::client_state::get_latest_client_state_update;
use crate::cli::OutputFormat;

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
    pub(crate) _output_format: OutputFormat,
}

/// Epoch information displayed to the user
#[derive(serde::Serialize)]
struct EpochInfo<'a> {
    epoch_index: u64,
    epoch_summary: &'a EpochSummary,
}

/// Checkpoint information displayed to the user
#[derive(serde::Serialize)]
struct CheckpointInfo<'a> {
    checkpoint_index: u64,
    checkpoint_commitment: &'a CheckpointCommitment,
    confirmation_status: &'a CheckpointConfStatus,
    proving_status: &'a CheckpointProvingStatus,
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
    if args.output_format == OutputFormat::Json {
        let epoch_info = EpochInfo {
            epoch_index: epoch_idx,
            epoch_summary: &epoch_summary,
        };
        println!("{}", serde_json::to_string_pretty(&epoch_info).unwrap());
    } else {
        println!("epoch_summary.epoch_index  {epoch_idx}");
        println!("epoch_summary.epoch {}", epoch_summary.epoch());

        let epoch_terminal = epoch_summary.terminal();
        println!("epoch_summary.terminal.slot {}", epoch_terminal.slot());
        println!("epoch_summary.terminal.blkid {:?}", epoch_terminal.blkid());

        let prev_termial = epoch_summary.prev_terminal();
        println!("epoch_summary.prev_terminal.slot {}", prev_termial.slot());
        println!(
            "epoch_summary.prev_terminal.blkid {:?}",
            prev_termial.blkid()
        );

        let new_l1_block = epoch_summary.new_l1();
        println!("epoch_summary.new_l1.height {}", new_l1_block.height());
        println!("epoch_summary.new_l1.blkid {:?}", new_l1_block.blkid());

        println!(
            "epoch_summary.final_state {:?}",
            epoch_summary.final_state()
        );
    }

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

    // Print checkpoint information
    if args.output_format == OutputFormat::Json {
        let checkpoint_info = CheckpointInfo {
            checkpoint_index: checkpoint_idx,
            checkpoint_commitment: entry.checkpoint.commitment(),
            proving_status: &entry.proving_status,
            confirmation_status: &entry.confirmation_status,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&checkpoint_info).unwrap()
        );
    } else {
        let batch_info = entry.checkpoint.batch_info();
        let batch_transition = entry.checkpoint.batch_transition();
        let confirmation_status = entry.confirmation_status;
        let proving_status = entry.proving_status;
        println!("checkpoint_index:  {checkpoint_idx}");
        println!("checkpoint.batch.epoch: {}", batch_info.epoch());
        println!(
            "checkpoint.batch.l1_range.start.height {}",
            batch_info.l1_range.0.height()
        );
        println!(
            "checkpoint.batch.l1_range.start.blkid {:?}",
            batch_info.l1_range.0.blkid()
        );
        println!(
            "checkpoint.batch.l1_range.end.height {}",
            batch_info.l1_range.1.height()
        );
        println!(
            "checkpoint.batch.l1_range.end.blkid {:?}",
            batch_info.l1_range.1.blkid()
        );
        println!(
            "checkpoint.batch.l2_range.start.slot {}",
            batch_info.l2_range.0.slot()
        );
        println!(
            "checkpoint.batch.l2_range.start.blkid {:?}",
            batch_info.l2_range.0.blkid()
        );
        println!(
            "checkpoint.batch.l2_range.end.slot {}",
            batch_info.l2_range.1.slot()
        );
        println!(
            "checkpoint.batch.l2_range.end.blkid {:?}",
            batch_info.l2_range.1.blkid()
        );

        println!(
            "checkpoint.batch_transition.chainstate.pre_root {:?}",
            batch_transition.chainstate_transition.pre_state_root
        );
        println!(
            "checkpoint.batch_transition.chainstate.post_root {:?}",
            batch_transition.chainstate_transition.post_state_root
        );
        println!(
            "checkpoint.batch_transition.tx_filter.pre_config_hash {:?}",
            batch_transition.tx_filters_transition.pre_config_hash
        );
        println!(
            "checkpoint.batch_transition.tx_filter.post_config_hash {:?}",
            batch_transition.tx_filters_transition.post_config_hash
        );

        match confirmation_status {
            CheckpointConfStatus::Pending => {
                println!("checkpoint.confirmation_status: {confirmation_status:?}");
            }
            CheckpointConfStatus::Confirmed(ref checkpoint_l1_ref) => {
                println!("checkpoint.confirmation_status: Confirmed");
                print_checkpoint_l1_ref(checkpoint_l1_ref);
            }
            CheckpointConfStatus::Finalized(ref checkpoint_l1_ref) => {
                println!("checkpoint.confirmation_status: Finalized");
                print_checkpoint_l1_ref(checkpoint_l1_ref);
            }
        }

        println!("checkpoint.proving_status: {proving_status:?}");
    }

    Ok(())
}

/// Print checkpoint's l1 refencence
fn print_checkpoint_l1_ref(l1ref: &CheckpointL1Ref) {
    println!(
        "checkpoint.confirmation_status.l1_ref.l1_commitment.height: {:?}",
        l1ref.l1_commitment.height()
    );
    println!(
        "checkpoint.confirmation_status.l1_ref.l1_commitment.blkid: {:?}",
        l1ref.l1_commitment.blkid()
    );
    println!(
        "checkpoint.confirmation_status.l1_ref.txid: {:?}",
        l1ref.txid
    );
    println!(
        "checkpoint.confirmation_status.l1_ref.wtxid: {:?}",
        l1ref.wtxid
    );
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

    println!(
        "checkpoints_summary.expected_checkpoints_count {}",
        last_idx + 1
    );
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
        "checkpoints_summary.checkpoints_found_in_db {}",
        checkpoint_commitments.len()
    );

    // Check if all checkpoints are present in L1 blocks
    let (l1_tip_height, _) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    let (client_state_update, _) = get_latest_client_state_update(db, None)?;
    let (client_state, _) = client_state_update.into_parts();
    let horizon_l1_height = client_state.horizon_l1_height();

    let mut found_checkpoints = 0;
    for l1_height in horizon_l1_height..=l1_tip_height {
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

    println!("checkpoints_summary.checkpoints_in_l1_blocks {found_checkpoints}");

    Ok(())
}
