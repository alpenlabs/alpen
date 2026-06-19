//! OL block data operation interface.

use strata_db_types::traits::*;
use strata_identifiers::{OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLBlockDatabase> => OLBlockOps, component = components::STORAGE_OL) {
        get_block_data(id: OLBlockId) => Option<OLBlock>;
        put_block_data(block: OLBlock) => ();
        get_block_high_watermark() => Option<OLBlockCommitment>;
        put_block_data_with_high_watermark(block: OLBlock) => OLBlockCommitment;
        clear_block_high_watermark(expected: OLBlockCommitment) => bool;
        rollback_block_high_watermark(target: OLBlockCommitment) => bool;
        del_block_data(id: OLBlockId) => bool;
        get_blocks_at_height(slot: u64) => Vec<OLBlockId>;
        get_canonical_block(slot: Slot) => Option<OLBlockId>;
        update_canonical_blocks_above(pivot_slot: Slot, blocks: Vec<(Slot, OLBlockId)>) => ();
        get_tip_slot() => Slot;
        get_block_status(id: OLBlockId) => Option<BlockStatus>;
        set_block_status(id: OLBlockId, status: BlockStatus) => bool;
    }
}
