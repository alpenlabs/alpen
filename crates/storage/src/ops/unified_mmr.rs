use strata_db_types::traits::UnifiedMmrDatabase;
use strata_identifiers::Hash;

use crate::exec::*;

inst_ops_simple! {
    (<D: UnifiedMmrDatabase> => UnifiedMmrDataOps) {
        append_leaf(mmr_id: Vec<u8>, hash: Hash) => u64;
        append_leaf_with_preimage(mmr_id: Vec<u8>, hash: Hash, preimage: Vec<u8>) => u64;
        get_preimage(mmr_id: Vec<u8>, index: u64) => Option<Vec<u8>>;
        pop_leaf(mmr_id: Vec<u8>) => Option<Hash>;
        get_node(mmr_id: Vec<u8>, pos: u64) => Hash;
        get_mmr_size(mmr_id: Vec<u8>) => u64;
        get_num_leaves(mmr_id: Vec<u8>) => u64;
    }
}
