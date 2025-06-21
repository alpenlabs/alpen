use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{ChainstateDatabase, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_rocksdb::CommonDb;
use strata_state::header::L2Header;

// use strata_state::header::L2Header;
use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to reset the chainstate to a specific L2 block.
#[derive(Args, Debug)]
pub struct ResetChainstateArgs {
    /// Target L2 block hash or number to roll back to.
    #[arg(value_name = "L2_BLOCK_ID")]
    pub block_id: String,

    /// Allow resetting to a non‑epoch‑terminal block (dangerous).
    #[arg(long = "allow-non-terminal")]
    pub allow_nterm: bool,
}

/// Reset the chainstate to a specific L2 block.
pub fn reset_chainstate(
    db: Arc<CommonDb>,
    args: ResetChainstateArgs,
) -> Result<(), DisplayedError> {
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let target_block_id: L2BlockId = Buf32::from(bytes).into();
    let target_block_data = db
        .l2_db()
        .get_block_data(target_block_id)
        .internal_error("Failed to read block data")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("block with id not found"),
                Box::new(target_block_id),
            )
        })?;
    let target_block_height = target_block_data.header().slot();

    let last_l2_write_idx = db
        .chain_state_db()
        .get_last_write_idx()
        .internal_error("Failed to fetch latest chainstate write index")?;

    let chainstate_entry = db
        .chain_state_db()
        .get_write_batch(last_l2_write_idx)
        .internal_error("Failed to fetch chainstate entry")?
        .expect("valid entry");
    let (batch_info, _) = chainstate_entry.to_parts();

    let finalized_height = batch_info
        .new_toplevel_state()
        .finalized_epoch()
        .last_slot();

    if target_block_height < finalized_height {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    Ok(())
}
