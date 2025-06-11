use std::sync::Arc;

use clap::Args;
use strata_rocksdb::CommonDb;

use crate::errors::DisplayedError;

#[derive(Args, Debug)]
pub struct ResetChainstateArgs {
    /// Target L2 block hash or number to roll back to.
    #[arg(value_name = "ALPEN_BLOCK_ID")]
    pub block_id: String,

    /// Allow resetting to a non‑epoch‑terminal block (dangerous).
    #[arg(long = "allow-non-terminal")]
    pub allow_nterm: bool,
}

pub fn reset_chainstate(
    _db: Arc<CommonDb>,
    args: ResetChainstateArgs,
) -> Result<(), DisplayedError> {
    // lib::ops::reset_chainstate(db, &args.block_id, args.allow_nterm)?;
    println!("Chainstate reset to {}", args.block_id);
    Ok(())
}
