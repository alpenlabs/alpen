use std::sync::Arc;

use clap::Args;
use strata_db::traits::{CheckpointDatabase, Database};
use strata_rocksdb::CommonDb;

use crate::errors::{DbtoolError, Result};

#[derive(Args, Debug)]
pub struct GetCheckpointDataArgs {
    /// Checkpoint index; defaults to the latest
    checkpoint_idx: Option<u64>,
}

pub fn get_checkpoint_data(db: Arc<CommonDb>, args: GetCheckpointDataArgs) -> Result<()> {
    // Determine checkpoint index
    let checkpoint_idx = match args.checkpoint_idx {
        Some(i) => i,
        None => db
            .checkpoint_db()
            .get_last_checkpoint_idx()
            .map_err(DbtoolError::from)? // convert DB error → DbtoolError
            .ok_or_else(|| DbtoolError::Db("no checkpoints found".into()))?,
    };

    // Fetch checkpoint data
    let entry = db
        .checkpoint_db()
        .get_checkpoint(checkpoint_idx)? // -> Option<CheckpointEntry>
        .ok_or_else(|| DbtoolError::Db(format!("checkpoint {checkpoint_idx} not found")))?;

    let checkpoint_commitment = entry.checkpoint.commitment();

    // Print checkpoint information
    println!("Checkpoint idx:  {checkpoint_idx}");
    println!("Checkpoint commitment: {checkpoint_commitment:#?}");
    println!("Checkpoint status: {:?}", entry.confirmation_status);
    println!("Proving status: {:?}", entry.proving_status);

    Ok(())
}
