//! L2 block data operation interface.

use std::sync::Arc;

use strata_db::traits::*;
use strata_state::{block::L2BlockBundle, id::L2BlockId};

use crate::exec::*;

inst_ops_simple! {
    (<D: L2BlockDatabase> => L2DataOps) {
        get_block_data(id: L2BlockId) => Option<L2BlockBundle>;
        get_blocks_at_height(h: u64) => Vec<L2BlockId>;
        get_block_status(id: L2BlockId) => Option<BlockStatus>;
        put_block_data(block: L2BlockBundle) => ();
        set_block_status(id: L2BlockId, status: BlockStatus) => ();
    }
}
