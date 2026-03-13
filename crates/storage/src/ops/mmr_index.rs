//! MMR index data operation interface.

use strata_db_types::{
    traits::MmrIndexDatabase, LeafPos, MmrBatchWrite, MmrNodePos, MmrNodeTable, NodePos, RawMmrId,
};
use strata_identifiers::Hash;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: MmrIndexDatabase> => MmrIndexOps, component = components::STORAGE_MMR_INDEX) {
        get_node(mmr_id: RawMmrId, pos: NodePos) => Option<Hash>;
        get_preimage(mmr_id: RawMmrId, pos: LeafPos) => Option<Vec<u8>>;
        get_leaf_count(mmr_id: RawMmrId) => u64;
        fetch_node_paths(nodes: Vec<MmrNodePos>, preimages: bool) => MmrNodeTable;
        apply_update(batch: MmrBatchWrite) => ();
    }
}
