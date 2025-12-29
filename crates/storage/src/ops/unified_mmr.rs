use strata_db_types::{mmr_helpers::MmrId, traits::UnifiedMmrDatabase};

use crate::exec::*;

inst_ops_simple! {
    (<D: UnifiedMmrDatabase> => UnifiedMmrDataOps) {
        append_leaf(mmr_id: MmrId, hash: [u8; 32]) => u64;
        append_leaf_with_preimage(mmr_id: MmrId, hash: [u8; 32], preimage: Vec<u8>) => u64;
        get_preimage(mmr_id: MmrId, index: u64) => Option<Vec<u8>>;
        pop_leaf(mmr_id: MmrId) => Option<[u8; 32]>;
        get_node(mmr_id: MmrId, pos: u64) => [u8; 32];
        get_mmr_size(mmr_id: MmrId) => u64;
        get_num_leaves(mmr_id: MmrId) => u64;
    }
}
