//! OL block data operation interface.

use strata_db_types::traits::*;
use strata_identifiers::{OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;

use crate::exec::*;

inst_ops_simple! {
    (<D: OLBlockDatabase> => OLBlockOps) {
        get_block_data(id: OLBlockId) => Option<OLBlock>;
        put_block_data(block: OLBlock) => ();
        del_block_data(id: OLBlockId) => bool;
        get_blocks_at_height(slot: u64) => Vec<OLBlockId>;
        get_tip_slot() => Slot;
        get_block_status(id: OLBlockId) => Option<BlockStatus>;
        set_block_status(id: OLBlockId, status: BlockStatus) => bool;
    }
}
