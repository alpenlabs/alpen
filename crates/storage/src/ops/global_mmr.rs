use strata_db_types::traits::GlobalMmrDatabase;
use strata_identifiers::{Hash, RawMmrId};

use crate::exec::*;

inst_ops_simple! {
    (<D: GlobalMmrDatabase> => GlobalMmrDataOps) {
        append_leaf(mmr_id: RawMmrId, hash: Hash) => u64;
        append_leaf_with_preimage(mmr_id: RawMmrId, hash: Hash, preimage: Vec<u8>) => u64;
        get_preimage(mmr_id: RawMmrId, index: u64) => Option<Vec<u8>>;
        pop_leaf(mmr_id: RawMmrId) => Option<Hash>;
        get_node(mmr_id: RawMmrId, pos: u64) => Option<Hash>;
        get_mmr_size(mmr_id: RawMmrId) => u64;
        get_num_leaves(mmr_id: RawMmrId) => u64;
    }
}
