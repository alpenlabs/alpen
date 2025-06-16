use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{Database, L1Database};
use strata_primitives::{
    buf::Buf32,
    l1::{L1BlockId, ProtocolOperation},
};
use strata_rocksdb::CommonDb;

use crate::errors::{DisplayableError, DisplayedError};

#[derive(Args, Debug)]
pub struct GetL1ManifestArgs {
    /// Block height; defaults to the chain tip
    #[arg(value_name = "L1_BLOCK_ID")]
    pub block_id: String,
}

pub fn get_l1_manifest(db: Arc<CommonDb>, args: GetL1ManifestArgs) -> Result<(), DisplayedError> {
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let block_id = L1BlockId::from(Buf32::from(bytes));

    // Get block manifest
    let l1_block_manifest = db
        .l1_db()
        .get_block_manifest(block_id)
        .internal_error("Failed to get block txs")?
        .unwrap();

    for tx in l1_block_manifest.txs().iter() {
        for proto_op in tx.protocol_ops().iter() {
            match proto_op {
                ProtocolOperation::Checkpoint(signed_checkpoint) => {
                    println!(
                        "checkpoint commitment: {:?}",
                        signed_checkpoint.checkpoint().commitment()
                    );
                }
                _ => continue,
            }
        }
    }

    Ok(())
}
