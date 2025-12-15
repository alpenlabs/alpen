//! MMR database operation interface.

use strata_db_types::traits::MmrDatabase;

use crate::exec::*;

inst_ops_simple! {
    (<D: MmrDatabase> => MmrDataOps) {
        append_leaf(hash: [u8; 32]) => u64;
        pop_leaf() => Option<[u8; 32]>;
        get_node(pos: u64) => [u8; 32];
        mmr_size() => u64;
        num_leaves() => u64;
    }
}
