use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::L1WriterDatabase;
use strata_primitives::buf::Buf32;

use super::checkpoint::{get_checkpoint_at_index, get_checkpoint_index_range};
use crate::{
    cli::OutputFormat,
    output::{
        l1_writer::{L1WriterPayloadInfo, L1WriterSummary},
        output,
    },
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-writer-summary")]
/// Get a summary of L1 writer database contents
pub(crate) struct GetL1WriterSummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-writer-payload")]
/// Get a specific L1 writer payload entry by index
pub(crate) struct GetL1WriterPayloadArgs {
    /// payload entry index
    #[argh(positional)]
    pub(crate) index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get a summary of L1 writer database contents
pub(crate) fn get_l1_writer_summary(
    db: &impl strata_db::traits::DatabaseBackend,
    args: GetL1WriterSummaryArgs,
) -> Result<(), DisplayedError> {
    let l1_writer_db = db.writer_db();

    // Get total counts
    let next_payload_idx = l1_writer_db
        .get_next_payload_idx()
        .internal_error("Failed to get next payload index")?;
    let next_intent_idx = l1_writer_db
        .get_next_intent_idx()
        .internal_error("Failed to get next intent index")?;

    // Check checkpoint to L1 writer mapping
    let (total_checkpoints, checkpoints_with_l1_entries, checkpoints_without_l1_entries) =
        if let Some((start_epoch, end_epoch)) = get_checkpoint_index_range(db)? {
            let total = end_epoch + 1;
            let mut with_entries = 0;
            let mut without_entries = 0;

            // Iterate through all checkpoint epochs
            for epoch in start_epoch..=end_epoch {
                if let Some(checkpoint_entry) = get_checkpoint_at_index(db, epoch)? {
                    let checkpoint_hash = checkpoint_entry.checkpoint.hash();

                    if l1_writer_db
                        .get_intent_by_id(checkpoint_hash)
                        .internal_error("Failed to get intent entry")?
                        .is_some()
                    {
                        with_entries += 1;
                    } else {
                        without_entries += 1;
                    }
                }
            }

            (total, with_entries, without_entries)
        } else {
            (0, 0, 0)
        };

    let summary = L1WriterSummary {
        total_payload_entries: next_payload_idx,
        total_intent_entries: next_intent_idx,
        total_checkpoints,
        checkpoints_with_l1_entries,
        checkpoints_without_l1_entries,
    };

    output(&summary, args.output_format)
}

/// Get a specific L1 writer payload entry by index
pub(crate) fn get_l1_writer_payload(
    db: &impl strata_db::traits::DatabaseBackend,
    args: GetL1WriterPayloadArgs,
) -> Result<(), DisplayedError> {
    let l1_writer_db = db.writer_db();

    if let Some(payload_entry) = l1_writer_db
        .get_payload_entry_by_idx(args.index)
        .internal_error("Failed to get payload entry")?
    {
        let payload_info = L1WriterPayloadInfo {
            index: args.index,
            status: payload_entry.status.clone(),
            payloads: payload_entry.payloads.clone(),
            commit_txid: if payload_entry.commit_txid == Buf32::zero() {
                None
            } else {
                Some(payload_entry.commit_txid)
            },
            reveal_txid: if payload_entry.reveal_txid == Buf32::zero() {
                None
            } else {
                Some(payload_entry.reveal_txid)
            },
        };
        output(&payload_info, args.output_format)
    } else {
        Err(DisplayedError::UserError(
            format!("No payload entry found at index {}", args.index),
            Box::new(args.index),
        ))
    }
}
