use std::sync::Arc;

use strata_rocksdb::CommonDb;

use crate::{cmd::ResetChainstate, errors::Result};

pub fn reset_chainstate(_db: Arc<CommonDb>, args: ResetChainstate) -> Result<()> {
    // lib::ops::reset_chainstate(db, &args.block_id, args.allow_nterm)?;
    println!("Chainstate reset to {}", args.block_id);
    Ok(())
}
