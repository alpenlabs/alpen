use std::sync::Arc;

use clap::Args;
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L2BlockDatabase};
use strata_rocksdb::CommonDb;

use crate::errors::{DbtoolError, Result};

#[derive(Args, Debug)]
pub struct GetAlpenBlockArgs {
    /// Block height; defaults to the chain tip
    #[arg(value_name = "ALPEN_BLOCK_HEIGHT")]
    pub block_height: Option<u64>,
}

pub fn get_alpen_block(db: Arc<CommonDb>, args: GetAlpenBlockArgs) -> Result<()> {
    // Determine block height
    let height = args
        .block_height
        .unwrap_or_else(|| db.chain_state_db().get_last_write_idx().unwrap_or(0));

    // Fetch all block-ids at that height
    let block_ids = db.l2_db().get_blocks_at_height(height)?;

    if block_ids.is_empty() {
        println!("No block found at height {height}");
        return Ok(());
    }

    // Print each block header and status
    for blk_id in block_ids {
        let status = db
            .l2_db()
            .get_block_status(blk_id)?
            .unwrap_or(BlockStatus::Unchecked);

        let bundle = db
            .l2_db()
            .get_block_data(blk_id)?
            .ok_or_else(|| DbtoolError::Db(format!("missing data for block {blk_id}")))?;

        println!("Alpen block {blk_id} – status: {status:?}");
        println!("{:#?}", bundle.block().header());
    }

    Ok(())
}
