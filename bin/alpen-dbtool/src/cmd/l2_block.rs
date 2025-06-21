use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{BlockStatus, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to show details about a specific L2 block.
#[derive(Args, Debug)]
pub struct GetL2BlockArgs {
    /// L2 Block id
    #[arg(value_name = "L2_BLOCK_ID")]
    pub block_id: String,
}

/// Show details about a specific L2 block.
pub fn get_l2_block(db: Arc<CommonDb>, args: GetL2BlockArgs) -> Result<(), DisplayedError> {
    // Convert String to L2BlockId
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

    // Fetch block status and data
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

    // Print status and header
    println!("L2 block id: {block_id:?}, status: {status:?}");
    println!("Block header: {:#?}", bundle.block().header());
    println!("L1 segment");
    for l1_manifest in bundle.block().body().l1_segment().new_manifests().iter() {
        println!(
            "L1 blkid {:?}, height {}, epoch {}, txs {}",
            l1_manifest.blkid(),
            l1_manifest.height(),
            l1_manifest.epoch(),
            l1_manifest.txs().len()
        );
    }

    Ok(())
}
