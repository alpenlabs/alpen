//! Extra utility functions for block proposal correctness checks.

use strata_ol_chain_types::{BlockCheckError, L2Block, L2Header, check_block_credential};
use strata_params::RollupParams;
use strata_primitives::prelude::*;

/// Checks a block's credential to ensure that it was authentically proposed.
pub fn check_block_proposal_valid(
    _blkid: &L2BlockId,
    block: &L2Block,
    params: &RollupParams,
) -> Result<(), BlockCheckError> {
    // If it's not the genesis block, check that the block is correctly signed.
    if block.header().slot() > 0 {
        check_block_credential(block.header(), params)?;
    }

    Ok(())
}
