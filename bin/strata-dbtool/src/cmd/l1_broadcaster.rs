use std::sync::Arc;

use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::{traits::L1BroadcastDatabase, types::L1TxStatus};

use crate::{
    cli::OutputFormat,
    output::{
        l1_broadcaster::{L1BroadcasterSummary, L1BroadcasterTxInfo},
        output,
    },
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-broadcaster-summary")]
/// Get summary of L1 broadcaster database
pub(crate) struct GetL1BroadcasterSummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-broadcaster-tx")]
/// Get L1 broadcaster transaction entry by index
pub(crate) struct GetL1BroadcasterTxArgs {
    /// transaction index
    #[argh(positional)]
    pub(crate) index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get summary of L1 broadcaster database
pub(crate) fn get_l1_broadcaster_summary(
    broadcast_db: Arc<impl L1BroadcastDatabase>,
    args: GetL1BroadcasterSummaryArgs,
) -> Result<(), DisplayedError> {
    let mut total_tx_entries = 0;
    let mut unpublished_count = 0;
    let mut published_count = 0;
    let mut confirmed_count = 0;
    let mut finalized_count = 0;
    let mut invalid_inputs_count = 0;

    // Get the next index to determine the range
    let next_idx = broadcast_db
        .get_next_tx_idx()
        .internal_error("Failed to get next tx index")?;

    // Iterate through all transaction entries to count by status
    for idx in 0..next_idx {
        if let Some(tx_entry) = broadcast_db
            .get_tx_entry(idx)
            .internal_error(format!("Failed to get tx entry at index {idx}"))?
        {
            total_tx_entries += 1;
            match tx_entry.status {
                L1TxStatus::Unpublished => unpublished_count += 1,
                L1TxStatus::Published => published_count += 1,
                L1TxStatus::Confirmed { .. } => confirmed_count += 1,
                L1TxStatus::Finalized { .. } => finalized_count += 1,
                L1TxStatus::InvalidInputs => invalid_inputs_count += 1,
            }
        }
    }

    let summary = L1BroadcasterSummary {
        total_tx_entries,
        unpublished_count,
        published_count,
        confirmed_count,
        finalized_count,
        invalid_inputs_count,
    };

    output(&summary, args.output_format)
}

/// Get L1 broadcaster transaction entry by index
pub(crate) fn get_l1_broadcaster_tx(
    broadcast_db: Arc<impl L1BroadcastDatabase>,
    args: GetL1BroadcasterTxArgs,
) -> Result<(), DisplayedError> {
    let tx_entry = match broadcast_db.get_tx_entry(args.index) {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            return Err(DisplayedError::UserError(
                format!("No tx entry found at index {}", args.index),
                Box::new(args.index),
            ));
        }
        Err(e) => {
            // Check if this is a "does not exist" error, which should be a user error
            let error_msg = format!("{e}");
            if error_msg.contains("Entry does not exist") || error_msg.contains("does not exist") {
                return Err(DisplayedError::UserError(
                    format!("No tx entry found at index {}", args.index),
                    Box::new(args.index),
                ));
            } else {
                return Err(DisplayedError::InternalError(
                    format!(
                        "Database error while getting tx entry at index {}",
                        args.index
                    ),
                    Box::new(e),
                ));
            }
        }
    };

    let tx_info = L1BroadcasterTxInfo {
        index: args.index,
        txid: match broadcast_db.get_txid(args.index) {
            Ok(Some(txid)) => txid,
            Ok(None) => {
                return Err(DisplayedError::UserError(
                    format!("No txid found for index {}", args.index),
                    Box::new(args.index),
                ));
            }
            Err(e) => {
                // Check if this is a "does not exist" error, which should be a user error
                let error_msg = format!("{e}");
                if error_msg.contains("Entry does not exist")
                    || error_msg.contains("does not exist")
                {
                    return Err(DisplayedError::UserError(
                        format!("No txid found for index {}", args.index),
                        Box::new(args.index),
                    ));
                } else {
                    return Err(DisplayedError::InternalError(
                        format!("Database error while getting txid for index {}", args.index),
                        Box::new(e),
                    ));
                }
            }
        },
        status: &tx_entry.status,
        raw_tx: tx_entry.tx_raw(),
    };

    output(&tx_info, args.output_format)
}
