use strata_db_types::{mmr_helpers::MmrId, traits::UnifiedMmrDatabase};

use crate::exec::*;

inst_ops_simple! {
    (<D: UnifiedMmrDatabase> => UnifiedMmrDataOps) {
        append_leaf(mmr_id: MmrId, hash: [u8; 32]) => u64;
        pop_leaf(mmr_id: MmrId) => Option<[u8; 32]>;
        get_node(mmr_id: MmrId, pos: u64) => [u8; 32];
        mmr_size(mmr_id: MmrId) => u64;
        num_leaves(mmr_id: MmrId) => u64;
    }
}
