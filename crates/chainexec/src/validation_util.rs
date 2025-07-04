//! Extra utility functions for things like structural validity checks.

use strata_primitives::prelude::*;
use strata_state::{
    block_validation::{check_block_credential, validate_block_segments},
    prelude::*,
};
use tracing::*;

/// Considers if the block is plausibly valid and if we should attach it to the
/// pending unfinalized blocks tree.  The block is assumed to already be
/// structurally consistent.
// TODO remove FCM arg from this
#[allow(dead_code)]
pub(crate) fn check_new_block(_blkid: &L2BlockId, block: &L2Block, params: &RollupParams) -> bool {
    // If it's not the genesis block, check that the block is correctly signed.
    if block.header().slot() > 0 {
        let cred_ok = check_block_credential(block.header(), params);
        if !cred_ok {
            warn!("block has invalid credential");
            return false;
        }
    }

    if !validate_block_segments(block) {
        return false;
    }

    true
}
