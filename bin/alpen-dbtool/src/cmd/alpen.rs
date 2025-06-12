use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{BlockStatus, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

#[derive(Args, Debug)]
pub struct GetAlpenBlockArgs {
    /// Block height; defaults to the chain tip
    #[arg(value_name = "ALPEN_BLOCK_ID")]
    pub block_id: String,
}

pub fn get_alpen_block(db: Arc<CommonDb>, args: GetAlpenBlockArgs) -> Result<(), DisplayedError> {
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let block_id = L2BlockId::from(Buf32::from(bytes));

    // Print block header and status
    let status = db
        .l2_db()
        .get_block_status(block_id)
        .internal_error("Failed to read block status")?
        .unwrap_or(BlockStatus::Unchecked);

    let bundle = db
        .l2_db()
        .get_block_data(block_id)
        .internal_error("Failed to read block data")?
        .ok_or_else(|| {
            DisplayedError::UserError(format!("block with id not found"), Box::new(block_id))
        })?;

    println!("Alpen block {block_id} – status: {status:?}");
    println!("{:#?}", bundle.block().header());

    Ok(())
}
