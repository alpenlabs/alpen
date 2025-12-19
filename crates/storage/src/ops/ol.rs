//! OL block data operation interface.

use strata_db_types::traits::*;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::OLBlock;

use crate::exec::*;

inst_ops_simple! {
    (<D: OLBlockDatabase> => OLBlockOps) {
        get_block_data(commitment: OLBlockCommitment) => Option<OLBlock>;
        put_block_data(commitment: OLBlockCommitment, block: OLBlock) => ();
        del_block_data(commitment: OLBlockCommitment) => ();
        get_blocks_at_height(slot: u64) => Vec<OLBlockId>;
        get_tip_block() => OLBlockId;
        get_block_status(id: OLBlockId) => Option<BlockStatus>;
        set_block_status(id: OLBlockId, status: BlockStatus) => ();
    }
}
