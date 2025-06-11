use std::sync::Arc;

use clap::Args;
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L1Database, L2BlockDatabase};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

#[derive(Args, Debug)]
pub struct GetSyncinfoArgs {
    /// Emit structured JSON instead of human‑readable output.
    #[arg(short = 'p', long = "porcelain")]
    porcelain: bool,
}

pub fn get_syncinfo(db: Arc<CommonDb>, _args: GetSyncinfoArgs) -> Result<(), DisplayedError> {
    let (l1_block_height, l1_block_id) = db
        .l1_db()
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .unwrap_or_default();

    let l2_block_height = db
        .chain_state_db()
        .get_last_write_idx()
        .internal_error("Failed to read L2 height")
        .ok();

    let l2_block_id = l2_block_height
        .and_then(|h| db.l2_db().get_blocks_at_height(h).ok())
        .and_then(|mut v| v.pop());

    let l2_block_status = l2_block_id
        .and_then(|id| db.l2_db().get_block_status(id).ok())
        .flatten();

    // Show sync information
    println!("L1 tip: {}, {}", l1_block_height, l1_block_id);
    match l2_block_height {
        Some(h) => println!(
            "L2 head : {}, {:?}  ({:?})",
            h,
            l2_block_id.unwrap(),
            l2_block_status.unwrap_or(BlockStatus::Unchecked)
        ),
        None => println!("L2 head : unknown (no writes yet)"),
    }

    Ok(())
}
