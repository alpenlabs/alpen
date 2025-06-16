use std::sync::Arc;

use clap::Args;
use strata_db::traits::{CheckpointDatabase, Database};
use strata_rocksdb::CommonDb;
use tracing::warn;

use crate::errors::{DisplayableError, DisplayedError};

#[derive(Args, Debug)]
pub struct GetEpochSummaryArgs {
    /// Epoch index; defaults to the latest
    epoch_idx: Option<u64>,
}

pub fn get_epoch_summary(
    db: Arc<CommonDb>,
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

    if epoch_commitments.len() == 0 {
        warn!("no epoch commitments founds");
        return Err(DisplayedError::UserError(
            format!("Invalid epoch index"),
            Box::new(epoch_idx),
        ));
    }

    let epoch_summary = db
        .checkpoint_db()
        .get_epoch_summary(*epoch_commitments.get(0).unwrap())
        .internal_error("Failed to fetch epoch summary")?
        .expect("a valid epoch summary");

    // Print epoch summary
    println!("Epoch idx:  {epoch_idx}");
    println!("Epoch summary: {epoch_summary:#?}");
    Ok(())
}
